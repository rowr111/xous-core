use utralib::*;

use crate::ifram::IframRange;
use crate::udma::*;
use crate::udma::{Bank, Udma};

const TIMEOUT_ITERS: usize = 1_000_000;

#[derive(Copy, Clone)]
#[repr(u8)]
pub enum I2cCmd {
    Start,
    WaitEvent(u8),
    Stop,
    RdAck,
    RdNack,
    WriteByte(u8),
    Write,
    Eot,
    // Wait cycles
    Wait(u8),
    // repeat times
    Repeat(u8),
    // clock divider
    Config(u16),
}
impl Into<u32> for I2cCmd {
    fn into(self) -> u32 {
        match self {
            I2cCmd::Start => 0x0000_0000,
            I2cCmd::WaitEvent(arg) => 0x1000_0000 | arg as u32,
            I2cCmd::Stop => 0x2000_0000,
            I2cCmd::RdAck => 0x4000_0000,
            I2cCmd::RdNack => 0x6000_0000,
            I2cCmd::WriteByte(arg) => 0x7000_0000 | arg as u32,
            I2cCmd::Write => 0x8000_0000,
            I2cCmd::Eot => 0x9000_0000,
            I2cCmd::Wait(arg) => 0xA000_0000 | arg as u32,
            I2cCmd::Repeat(arg) => 0xC000_0000 | arg as u32,
            I2cCmd::Config(divider) => 0xE000_0000 | divider as u32,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum I2cChannel {
    Channel0,
    Channel1,
    Channel2,
    Channel3,
}

enum I2cPending {
    Idle,
    Write(usize),
    Read(usize),
}
impl I2cPending {
    fn take(&mut self) -> I2cPending { core::mem::replace(self, I2cPending::Idle) }
}

pub trait I2cApi {
    fn i2c_write(&mut self, dev: u8, adr: u8, data: &[u8]) -> Result<usize, xous::Error>;

    /// initiate an i2c read. The read buffer is passed during the await.
    fn i2c_read(
        &mut self,
        dev: u8,
        adr: u8,
        buf: &mut [u8],
        repeated_start: bool,
    ) -> Result<usize, xous::Error>;
}

const MAX_I2C_TXLEN: usize = 512;
const MAX_I2C_RXLEN: usize = 512;
const MAX_I2C_CMDLEN: usize = 512;

pub struct I2c {
    csr: CSR<u32>,
    _channel: I2cChannel,
    divider: u16,
    perclk_freq: u32,
    pub ifram: IframRange,
    tx_buf: &'static mut [u8],
    tx_buf_phys: &'static mut [u8],
    rx_buf: &'static mut [u8],
    rx_buf_phys: &'static mut [u8],
    cmd_buf: &'static mut [u32],
    cmd_buf_phys: &'static mut [u32],
    seq_len: usize,
    pending: I2cPending,
}

impl Udma for I2c {
    fn csr_mut(&mut self) -> &mut CSR<u32> { &mut self.csr }

    fn csr(&self) -> &CSR<u32> { &self.csr }
}

impl I2c {
    /// Safety: called only after global clock for I2C channel is enabled.
    /// It is also unsafe to `Drop` because you have to cleanup the clock manually.
    #[cfg(feature = "std")]
    pub unsafe fn new(channel: I2cChannel, i2c_freq: u32, perclk_freq: u32) -> Option<Self> {
        // one page is the minimum size we can request
        if let Some(ifram) = IframRange::request(4096, None) {
            Some(I2c::new_with_ifram(channel, i2c_freq, perclk_freq, ifram))
        } else {
            None
        }
    }

    pub unsafe fn new_with_ifram(
        channel: I2cChannel,
        i2c_freq: u32,
        perclk_freq: u32,
        ifram: IframRange,
    ) -> Self {
        // divide-by-4 is an empirical observation
        let divider: u16 = ((((perclk_freq / 2) / i2c_freq) / 4).min(u16::MAX as u32)) as u16;
        // now setup the channel
        let base_addr = match channel {
            I2cChannel::Channel0 => utra::udma_i2c_0::HW_UDMA_I2C_0_BASE,
            I2cChannel::Channel1 => utra::udma_i2c_1::HW_UDMA_I2C_1_BASE,
            I2cChannel::Channel2 => utra::udma_i2c_2::HW_UDMA_I2C_2_BASE,
            I2cChannel::Channel3 => utra::udma_i2c_3::HW_UDMA_I2C_3_BASE,
        };
        #[cfg(target_os = "xous")]
        let csr_range = xous::syscall::map_memory(
            xous::MemoryAddress::new(base_addr),
            None,
            4096,
            xous::MemoryFlags::R | xous::MemoryFlags::W,
        )
        .expect("couldn't map i2c port");
        #[cfg(target_os = "xous")]
        let mut csr = CSR::new(csr_range.as_mut_ptr() as *mut u32);
        #[cfg(not(target_os = "xous"))]
        let mut csr = CSR::new(base_addr as *mut u32);
        // reset the block
        csr.wfo(utra::udma_i2c_0::REG_SETUP_R_DO_RST, 1);
        csr.wo(utra::udma_i2c_0::REG_SETUP, 0);
        // one page is the minimum size we can request
        let ifram_base = ifram.virt_range.as_ptr() as usize;
        let ifram_base_phys = ifram.phys_range.as_ptr() as usize;
        let mut i2c = I2c {
            csr,
            _channel: channel,
            ifram,
            divider,
            cmd_buf: unsafe { core::slice::from_raw_parts_mut(ifram_base as *mut u32, MAX_I2C_CMDLEN) },
            cmd_buf_phys: unsafe {
                core::slice::from_raw_parts_mut(ifram_base_phys as *mut u32, MAX_I2C_CMDLEN)
            },
            tx_buf: unsafe {
                core::slice::from_raw_parts_mut(
                    (ifram_base + MAX_I2C_CMDLEN * core::mem::size_of::<u32>()) as *mut u8,
                    MAX_I2C_TXLEN,
                )
            },
            tx_buf_phys: unsafe {
                core::slice::from_raw_parts_mut(
                    (ifram_base_phys + MAX_I2C_CMDLEN * core::mem::size_of::<u32>()) as *mut u8,
                    MAX_I2C_TXLEN,
                )
            },
            rx_buf: unsafe {
                core::slice::from_raw_parts_mut(
                    (ifram_base + MAX_I2C_TXLEN + MAX_I2C_CMDLEN * core::mem::size_of::<u32>()) as *mut u8,
                    MAX_I2C_RXLEN,
                )
            },
            rx_buf_phys: unsafe {
                core::slice::from_raw_parts_mut(
                    (ifram_base_phys + MAX_I2C_TXLEN + MAX_I2C_CMDLEN * core::mem::size_of::<u32>())
                        as *mut u8,
                    MAX_I2C_RXLEN,
                )
            },
            seq_len: 0,
            pending: I2cPending::Idle,
            perclk_freq,
        };
        crate::println!("Set divider to {}", divider,);
        i2c.send_cmd_list(&[I2cCmd::Config(divider)]);
        i2c
    }

    pub fn set_freq(&mut self, freq_hz: u32) {
        self.divider = ((((self.perclk_freq / 2) / freq_hz) / 4).min(u16::MAX as u32)) as u16;
    }

    // always blocks
    fn send_cmd_list(&mut self, cmds: &[I2cCmd]) {
        assert!(cmds.len() < MAX_I2C_CMDLEN);
        for (cmd, dst) in cmds.iter().zip(self.cmd_buf.iter_mut()) {
            *dst = (*cmd).into();
        }
        // safety: this is safe because the cmd_buf_phys() slice is passed to a function that only
        // uses it as a base/bounds reference and it will not actually access the data.
        unsafe {
            self.udma_enqueue(Bank::Custom, &self.cmd_buf_phys[..cmds.len()], CFG_EN);
        }
        while self.udma_busy(Bank::Custom) {}
    }

    fn push_cmd(&mut self, cmd: I2cCmd) {
        self.cmd_buf[self.seq_len] = cmd.into();
        self.seq_len += 1;
    }

    fn new_tranaction(&mut self) { self.seq_len = 0; }

    /// Returns a ShareViolation if the pending operation is a read, but `rx_buf` is `None`.
    pub fn i2c_await(&mut self, rx_buf: Option<&mut [u8]>, _use_yield: bool) -> Result<usize, xous::Error> {
        let mut timeout = 0;
        while self.busy() {
            timeout += 1;
            if timeout > TIMEOUT_ITERS {
                // reset the block
                self.udma_reset(Bank::Custom);
                self.udma_reset(Bank::Tx);
                self.udma_reset(Bank::Rx);
                self.csr.wfo(utra::udma_i2c_0::REG_SETUP_R_DO_RST, 1);
                self.csr.wo(utra::udma_i2c_0::REG_SETUP, 0);

                self.send_cmd_list(&[I2cCmd::Config(self.divider)]);
                self.pending.take();
                return Err(xous::Error::Timeout);
            }
            #[cfg(feature = "std")]
            if _use_yield {
                xous::yield_slice();
            }
        }
        let ret = match self.pending.take() {
            I2cPending::Read(len) => {
                if let Some(buf) = rx_buf {
                    buf[..len].copy_from_slice(&self.rx_buf[3..3 + len]);
                    Ok(len)
                } else {
                    // the pending transaction was a read, but the user did not call us
                    // as if we were a read.
                    Err(xous::Error::ShareViolation)
                }
            }
            I2cPending::Write(len) => Ok(len),
            I2cPending::Idle => Err(xous::Error::UseBeforeInit),
        };
        ret
    }

    fn busy(&self) -> bool {
        self.csr.rf(utra::udma_i2c_0::REG_STATUS_R_BUSY) != 0
            || self.csr.rf(utra::udma_i2c_0::REG_STATUS_R_BUSY) != 0
            || self.udma_busy(Bank::Custom)
            || self.udma_busy(Bank::Tx)
            || self.udma_busy(Bank::Rx)
    }

    pub fn i2c_write_async(&mut self, dev: u8, adr: u8, data: &[u8]) -> Result<usize, xous::Error> {
        // The implementation of this is gross because we have to stuff the command list
        assert!(data.len() < 256); // is is a conservative bound, the limit is due to the cmd buf length limit
        // into the pre-allocated Tx buf
        self.new_tranaction();
        self.push_cmd(I2cCmd::Config(self.divider));
        self.push_cmd(I2cCmd::Start);
        self.push_cmd(I2cCmd::WriteByte(dev << 1));
        self.push_cmd(I2cCmd::WriteByte(adr));
        for _ in 0..data.len() {
            self.push_cmd(I2cCmd::Write);
        }
        self.push_cmd(I2cCmd::Stop);

        self.tx_buf[..data.len()].copy_from_slice(&data);

        // safety: this is safe because the cmd_buf_phys() slice is passed to a function that only
        // uses it as a base/bounds reference and it will not actually access the data.
        unsafe {
            self.udma_enqueue(Bank::Tx, &self.tx_buf_phys[..data.len()], CFG_EN);
            self.udma_enqueue(Bank::Custom, &self.cmd_buf_phys[..self.seq_len], CFG_EN);
        }
        self.pending = I2cPending::Write(data.len());
        Ok(data.len())
    }

    /// initiate an i2c read. The read buffer is passed during the await.
    pub fn i2c_read_async(
        &mut self,
        dev: u8,
        adr: u8,
        len: usize,
        repeated_start: bool,
    ) -> Result<usize, xous::Error> {
        assert!(len < 256); // this is a conservative bound, actual limit is about 512 - 3 bytes

        // block has to be reset on every start transaction due to a... bug? programming error?
        // where the Rx length is mismatched from the actual length of Rx data expected because
        // it seems the Rx buffer pointer increments even during Tx events.
        self.csr.wfo(utra::udma_i2c_0::REG_SETUP_R_DO_RST, 1);
        self.csr.wo(utra::udma_i2c_0::REG_SETUP, 0);

        // into the pre-allocated Tx buf
        self.new_tranaction();
        self.push_cmd(I2cCmd::Config(self.divider));
        self.push_cmd(I2cCmd::Start);
        self.push_cmd(I2cCmd::WriteByte((dev << 1) | 0)); // specify write mode to send the read address
        self.push_cmd(I2cCmd::WriteByte(adr));
        if !repeated_start {
            self.push_cmd(I2cCmd::Stop);
        }
        self.push_cmd(I2cCmd::Start);
        self.push_cmd(I2cCmd::WriteByte((dev << 1) | 1)); // specify read mode to get the data
        for _ in 1..len {
            self.push_cmd(I2cCmd::RdAck);
        }
        self.push_cmd(I2cCmd::RdNack);
        self.push_cmd(I2cCmd::Stop);
        // safety: this is safe because the cmd_buf_phys() slice is passed to a function that only
        // uses it as a base/bounds reference and it will not actually access the data.
        unsafe {
            // the extra 3 are dummy bytes that were received while the address was being set up
            self.udma_enqueue(Bank::Rx, &self.rx_buf_phys[..len + 3], CFG_EN);
            self.udma_enqueue(Bank::Custom, &self.cmd_buf_phys[..self.seq_len], CFG_EN);
        }
        self.pending = I2cPending::Read(len);
        Ok(len)
    }
}

impl I2cApi for I2c {
    fn i2c_read(
        &mut self,
        dev: u8,
        adr: u8,
        buf: &mut [u8],
        repeated_start: bool,
    ) -> Result<usize, xous::Error> {
        self.i2c_read_async(dev, adr, buf.len(), repeated_start)?;
        self.i2c_await(Some(buf), true)
    }

    fn i2c_write(&mut self, dev: u8, adr: u8, data: &[u8]) -> Result<usize, xous::Error> {
        self.i2c_write_async(dev, adr, data)?;
        self.i2c_await(None, true)
    }
}
