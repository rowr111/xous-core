#![cfg_attr(target_os = "none", no_std)]

pub mod api;
use com::{Ipv4Conf, SsidRecord};
use num_traits::*;
use xous::{CID, Message, send_message};
use xous_ipc::Buffer;

pub mod protocols;
pub use api::*;
pub use smoltcp::time::Duration;
pub use smoltcp::wire::IpEndpoint;

/// NetConn is a crate-level structure that just counts the number of connections from this process to
/// the Net server. It's not mean to be created by user-facing code, so the visibility is (crate).
#[derive(Debug)]
pub(crate) struct NetConn {
    conn: CID,
}
impl NetConn {
    pub(crate) fn new(xns: &xous_names::XousNames) -> Result<Self, xous::Error> {
        REFCOUNT.fetch_add(1, Ordering::Relaxed);
        let conn =
            xns.request_connection_blocking(api::SERVER_NAME_NET).expect("Can't connect to Net server");
        Ok(NetConn { conn })
    }

    pub(crate) fn conn(&self) -> CID { self.conn }
}

use core::sync::atomic::{AtomicU32, Ordering};
static REFCOUNT: AtomicU32 = AtomicU32::new(0);
impl Drop for NetConn {
    fn drop(&mut self) {
        // the connection to the server side must be reference counted, so that multiple instances of this
        // object within a single process do not end up de-allocating the CID on other threads before
        // they go out of scope. Note to future me: you want this. Don't get rid of it because you
        // think, "nah, nobody will ever make more than one copy of this object".
        if REFCOUNT.fetch_sub(1, Ordering::Relaxed) == 1 {
            unsafe {
                xous::disconnect(self.conn).unwrap();
            }
        }
        // if there was object-specific state (such as a one-time use server for async callbacks, specific to
        // the object instance), de-allocate those items here. They don't need a reference count
        // because they are object-specific
    }
}

#[derive(Debug)]
pub struct NetManager {
    netconn: NetConn,
    wifi_state_cid: Option<CID>,
    wifi_state_sid: Option<xous::SID>,
}
impl NetManager {
    pub fn new() -> NetManager {
        NetManager {
            netconn: NetConn::new(&xous_names::XousNames::new().unwrap())
                .expect("can't connect to Net Server"),
            wifi_state_cid: None,
            wifi_state_sid: None,
        }
    }

    /// Sets the debug level for the network connection.
    ///
    /// This function sends a message to the network server to set the debug level. The debug level
    /// determines the verbosity of the logging output. The supported levels are:
    /// - `log::LevelFilter::Info`: Sets the debug level to Info.
    /// - `log::LevelFilter::Debug`: Sets the debug level to Debug.
    /// - `log::LevelFilter::Trace`: Sets the debug level to Trace.
    /// Any other level will default to Info.
    ///
    /// # Arguments
    ///
    /// * `level` - The desired debug level as a `log::LevelFilter`.
    ///
    /// # Example
    /// ```
    /// use log::LevelFilter;
    /// use net::Net; // Assuming `Net` is the struct that contains `set_debug_level`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// net.set_debug_level(LevelFilter::Debug); // Set the debug level to Debug
    /// ```
    pub fn set_debug_level(&self, level: log::LevelFilter) {
        let code = match level {
            log::LevelFilter::Info => 0,
            log::LevelFilter::Debug => 1,
            log::LevelFilter::Trace => 2,
            _ => 0,
        };
        send_message(
            self.netconn.conn(),
            Message::new_scalar(Opcode::SetDebug.to_usize().unwrap(), code, 0, 0, 0),
        )
        .expect("couldn't set debug");
    }

    /// Retrieves the IPv4 configuration for the network connection.
    ///
    /// This function sends a message to the network server to get the current IPv4 configuration.
    /// The configuration is returned as an `Option<Ipv4Conf>`, where `None` indicates that the
    /// configuration could not be retrieved.
    ///
    /// # Returns
    ///
    /// * `Option<Ipv4Conf>` - The current IPv4 configuration, or `None` if the configuration could not be
    ///   retrieved.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `get_ipv4_config`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// if let Some(config) = net.get_ipv4_config() {
    ///     println!("IPv4 Configuration: {:?}", config);
    /// } else {
    ///     println!("Failed to retrieve IPv4 configuration");
    /// }
    /// ```
    pub fn get_ipv4_config(&self) -> Option<Ipv4Conf> {
        let storage = Some(Ipv4Conf::default().encode_u16());
        let mut buf = Buffer::into_buf(storage).expect("Couldn't convert to memory structure");
        buf.lend_mut(self.netconn.conn(), Opcode::GetIpv4Config.to_u32().unwrap())
            .expect("Couldn't execute GetIpv4Config opcode");
        let maybe_config = buf.to_original().expect("couldn't restore config structure");
        if let Some(config) = maybe_config {
            let ipv4 = Ipv4Conf::decode_u16(&config);
            Some(ipv4)
        } else {
            None
        }
    }

    /// Resets the network connection.
    ///
    /// This function sends a message to the network server to reset the network connection. The reset
    /// operation is blocking, meaning the function will wait until the network server confirms that the
    /// reset has been completed.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `reset`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// net.reset(); // Reset the network connection
    /// ```
    pub fn reset(&self) {
        send_message(
            self.netconn.conn(),
            Message::new_blocking_scalar(Opcode::Reset.to_usize().unwrap(), 0, 0, 0, 0),
        )
        .expect("couldn't send reset");
    }

    /// Subscribes to WiFi state updates.
    ///
    /// This function sets up a subscription to receive WiFi state updates from the connection manager.
    /// It creates a one-time server to handle the subscription and forwards the updates to the specified
    /// return connection ID and opcode. This function should be used to check the WiFi state by reading
    /// the cached value from the connection manager, as direct calls to the COM could cause too much
    /// congestion.
    ///
    /// # Arguments
    ///
    /// * `return_cid` - The connection ID to which the WiFi state updates should be forwarded.
    /// * `opcode` - The opcode to use when forwarding the WiFi state updates.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the subscription is set up successfully, or an error
    ///   if it fails.
    ///
    /// # Example
    /// ```
    /// use xous::{CID, connect, create_server};
    /// use net::Net; // Assuming `Net` is the struct that contains `wifi_state_subscribe`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// let return_cid = connect(create_server().unwrap()).unwrap();
    /// let opcode = 1234; // Example opcode
    /// match net.wifi_state_subscribe(return_cid, opcode) {
    ///     Ok(()) => println!("Subscribed to WiFi state updates successfully"),
    ///     Err(e) => eprintln!("Failed to subscribe to WiFi state updates: {:?}", e),
    /// }
    /// ```
    pub fn wifi_state_subscribe(&mut self, return_cid: CID, opcode: u32) -> Result<(), xous::Error> {
        if self.wifi_state_cid.is_none() {
            let onetime_sid = xous::create_server().unwrap();
            let sub = WifiStateSubscription { sid: onetime_sid.to_array(), opcode };
            let buf = Buffer::into_buf(sub).or(Err(xous::Error::InternalError))?;
            buf.send(self.netconn.conn(), Opcode::SubscribeWifiStats.to_u32().unwrap())
                .or(Err(xous::Error::InternalError))?;
            self.wifi_state_cid = Some(xous::connect(onetime_sid).unwrap());
            self.wifi_state_sid = Some(onetime_sid);
            let _ = std::thread::spawn({
                let onetime_sid = onetime_sid.clone();
                let opcode = opcode.clone();
                move || {
                    loop {
                        let msg = xous::receive_message(onetime_sid).unwrap();
                        match FromPrimitive::from_usize(msg.body.id()) {
                            Some(WifiStateCallback::Update) => {
                                let buffer = unsafe {
                                    Buffer::from_memory_message(msg.body.memory_message().unwrap())
                                };
                                log::debug!("got state_subscribe callback {} {}", return_cid, opcode);
                                // have to transform it through the local memory space because you can't
                                // re-lend pages
                                let sub = buffer.to_original::<com::WlanStatusIpc, _>().unwrap();
                                let buf = Buffer::into_buf(sub).expect("couldn't convert to memory message");
                                buf.lend(return_cid, opcode).expect("couldn't forward state update");
                            }
                            Some(WifiStateCallback::Drop) => {
                                xous::return_scalar(msg.sender, 1).unwrap();
                                break;
                            }
                            _ => {
                                log::error!("got unknown opcode: {:?}", msg);
                            }
                        }
                    }
                    log::info!("destroying callback server");
                    xous::destroy_server(onetime_sid).unwrap();
                }
            });
            Ok(())
        } else {
            // you can only hook this once per object
            Err(xous::Error::ServerExists)
        }
    }

    /// Unsubscribes from WiFi state updates.
    ///
    /// This function unsubscribes from WiFi state updates by sending a message to the network server.
    /// If the subscription exists, it will be removed and the associated resources will be cleaned up.
    /// If there is no existing subscription, the function returns without error.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the unsubscription is successful, or an error if it
    ///   fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `wifi_state_unsubscribe`
    ///
    /// let mut net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.wifi_state_unsubscribe() {
    ///     Ok(()) => println!("Unsubscribed from WiFi state updates successfully"),
    ///     Err(e) => eprintln!("Failed to unsubscribe from WiFi state updates: {:?}", e),
    /// }
    /// ```
    pub fn wifi_state_unsubscribe(&mut self) -> Result<(), xous::Error> {
        if let Some(handler) = self.wifi_state_cid.take() {
            if let Some(sid) = self.wifi_state_sid.take() {
                let s = sid.to_array();
                send_message(
                    self.netconn.conn(),
                    Message::new_blocking_scalar(
                        Opcode::UnsubWifiStats.to_usize().unwrap(),
                        s[0] as usize,
                        s[1] as usize,
                        s[2] as usize,
                        s[3] as usize,
                    ),
                )
                .expect("couldn't unsubscribe");
            }
            send_message(
                handler,
                Message::new_blocking_scalar(WifiStateCallback::Drop.to_usize().unwrap(), 0, 0, 0, 0),
            )
            .ok();
            unsafe { xous::disconnect(handler).ok() };
        }
        Ok(())
    }

    /// Retrieves the list of available WiFi SSIDs.
    ///
    /// This function sends a message to the network server to fetch the list of available WiFi SSIDs.
    /// The function returns a tuple containing a vector of `SsidRecord` and the current `ScanState`.
    ///
    /// # Returns
    ///
    /// * `Result<(Vec<SsidRecord>, ScanState), xous::Error>` - Returns a tuple containing a vector of
    ///   `SsidRecord` and the current `ScanState` if successful, or an error if it fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `wifi_get_ssid_list`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.wifi_get_ssid_list() {
    ///     Ok((ssid_list, scan_state)) => {
    ///         println!("SSID List: {:?}", ssid_list);
    ///         println!("Scan State: {:?}", scan_state);
    ///     }
    ///     Err(e) => eprintln!("Failed to retrieve SSID list: {:?}", e),
    /// }
    /// ```
    pub fn wifi_get_ssid_list(&self) -> Result<(Vec<SsidRecord>, ScanState), xous::Error> {
        let alloc = SsidList::default();
        let mut buf = Buffer::into_buf(alloc).map_err(|_| xous::Error::InternalError)?;
        buf.lend_mut(self.netconn.conn(), Opcode::FetchSsidList.to_u32().unwrap())?;
        let ssid_list = buf.to_original::<SsidList, _>().map_err(|_| xous::Error::InternalError)?;
        let mut ret = Vec::<SsidRecord>::new();
        for maybe_item in ssid_list.list.iter() {
            if let Some(item) = maybe_item {
                ret.push(item.clone());
            }
        }
        Ok((ret, ssid_list.state))
    }

    /// Stops the connection manager.
    ///
    /// This function sends a message to the network server to stop the connection manager. The connection
    /// manager is responsible for managing network connections, and stopping it will halt all network
    /// activities.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the connection manager is stopped successfully, or
    ///   an error if it fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `connection_manager_stop`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.connection_manager_stop() {
    ///     Ok(()) => println!("Connection manager stopped successfully"),
    ///     Err(e) => eprintln!("Failed to stop connection manager: {:?}", e),
    /// }
    /// ```
    pub fn connection_manager_stop(&self) -> Result<(), xous::Error> {
        send_message(
            self.netconn.conn(),
            Message::new_scalar(Opcode::ConnMgrStartStop.to_usize().unwrap(), 0, 0, 0, 0),
        )
        .map(|_| ())
    }

    /// Starts the connection manager.
    ///
    /// This function sends a message to the network server to start the connection manager. The connection
    /// manager is responsible for managing network connections, and starting it will enable network
    /// activities.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the connection manager is started successfully, or
    ///   an error if it fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `connection_manager_run`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.connection_manager_run() {
    ///     Ok(()) => println!("Connection manager started successfully"),
    ///     Err(e) => eprintln!("Failed to start connection manager: {:?}", e),
    /// }
    /// ```
    pub fn connection_manager_run(&self) -> Result<(), xous::Error> {
        send_message(
            self.netconn.conn(),
            Message::new_scalar(Opcode::ConnMgrStartStop.to_usize().unwrap(), 1, 0, 0, 0),
        )
        .map(|_| ())
    }

    /// Turns off WiFi and stops the connection manager.
    ///
    /// This function sends a message to the network server to turn off WiFi and stop the connection manager.
    /// The connection manager is responsible for managing network connections, and stopping it will halt all
    /// network activities.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the operation is successful, or an error if it
    ///   fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `connection_manager_wifi_off_and_stop`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.connection_manager_wifi_off_and_stop() {
    ///     Ok(()) => println!("WiFi turned off and connection manager stopped successfully"),
    ///     Err(e) => eprintln!("Failed to turn off WiFi and stop connection manager: {:?}", e),
    /// }
    /// ```
    pub fn connection_manager_wifi_off_and_stop(&self) -> Result<(), xous::Error> {
        send_message(
            self.netconn.conn(),
            Message::new_scalar(Opcode::ConnMgrStartStop.to_usize().unwrap(), 2, 0, 0, 0),
        )
        .map(|_| ())
    }

    /// Turns on WiFi and starts the connection manager.
    ///
    /// This function sends a message to the network server to turn on WiFi and start the connection manager.
    /// The connection manager is responsible for managing network connections, and starting it will enable
    /// network activities.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the operation is successful, or an error if it
    ///   fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `connection_manager_wifi_on_and_run`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.connection_manager_wifi_on_and_run() {
    ///     Ok(()) => println!("WiFi turned on and connection manager started successfully"),
    ///     Err(e) => eprintln!("Failed to turn on WiFi and start connection manager: {:?}", e),
    /// }
    /// ```
    pub fn connection_manager_wifi_on_and_run(&self) -> Result<(), xous::Error> {
        send_message(
            self.netconn.conn(),
            Message::new_scalar(Opcode::ConnMgrStartStop.to_usize().unwrap(), 3, 0, 0, 0),
        )
        .map(|_| ())
    }

    /// Turns on WiFi.
    ///
    /// This function sends a message to the network server to turn on WiFi. This will enable WiFi
    /// connectivity without starting the connection manager.
    ///
    /// # Returns
    ///
    /// * `Result<(), xous::Error>` - Returns `Ok(())` if the operation is successful, or an error if it
    ///   fails.
    ///
    /// # Example
    /// ```
    /// use net::Net; // Assuming `Net` is the struct that contains `connection_manager_wifi_on`
    ///
    /// let net = Net::new(); // Assuming `Net::new()` initializes the network connection
    /// match net.connection_manager_wifi_on() {
    ///     Ok(()) => println!("WiFi turned on successfully"),
    ///     Err(e) => eprintln!("Failed to turn on WiFi: {:?}", e),
    /// }
    /// ```
    pub fn connection_manager_wifi_on(&self) -> Result<(), xous::Error> {
        send_message(
            self.netconn.conn(),
            Message::new_scalar(Opcode::ConnMgrStartStop.to_usize().unwrap(), 4, 0, 0, 0),
        )
        .map(|_| ())
    }
}
impl Drop for NetManager {
    fn drop(&mut self) { self.wifi_state_unsubscribe().unwrap(); }
}
