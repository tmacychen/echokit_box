use std::sync::{Arc, Mutex};

use esp_idf_svc::eventloop::EspSystemEventLoop;

mod app;
mod audio;
mod network;
mod protocol;
mod ui;
mod ws;

#[derive(Debug, Clone)]
struct Setting {
    ssid: String,
    pass: String,
    server_url: String,
}

mod bt {
    use std::sync::{Arc, Mutex};

    use esp32_nimble::{utilities::BleUuid, uuid128, BLEAdvertisementData, NimbleProperties};

    const SERVICE_ID: BleUuid = uuid128!("623fa3e2-631b-4f8f-a6e7-a7b09c03e7e0");
    const SSID_ID: BleUuid = uuid128!("1fda4d6e-2f14-42b0-96fa-453bed238375");
    const PASS_ID: BleUuid = uuid128!("a987ab18-a940-421a-a1d7-b94ee22bccbe");
    const SERVER_URL_ID: BleUuid = uuid128!("cef520a9-bcb5-4fc6-87f7-82804eee2b20");

    pub fn bt(
        setting: Arc<Mutex<(super::Setting, esp_idf_svc::nvs::EspDefaultNvs)>>,
    ) -> anyhow::Result<()> {
        let ble_device = esp32_nimble::BLEDevice::take();
        let ble_addr = ble_device.get_addr()?.to_string();
        let ble_advertising = ble_device.get_advertising();

        let server = ble_device.get_server();
        server.on_connect(|server, desc| {
            log::info!("Client connected: {:?}", desc);

            server
                .update_conn_params(desc.conn_handle(), 24, 48, 0, 60)
                .unwrap();

            if server.connected_count() < (esp_idf_svc::sys::CONFIG_BT_NIMBLE_MAX_CONNECTIONS as _)
            {
                log::info!("Multi-connect support: start advertising");
                ble_advertising.lock().start().unwrap();
            }
        });

        server.on_disconnect(|_desc, reason| {
            log::info!("Client disconnected ({:?})", reason);
        });

        let service = server.create_service(SERVICE_ID);

        let setting1 = setting.clone();
        let setting2 = setting.clone();

        let ssid_characteristic = service
            .lock()
            .create_characteristic(SSID_ID, NimbleProperties::READ | NimbleProperties::WRITE);
        ssid_characteristic
            .lock()
            .on_read(move |c, _| {
                log::info!("Read from SSID characteristic");
                let setting = setting1.lock().unwrap();
                c.set_value(setting.0.ssid.as_bytes());
            })
            .on_write(move |args| {
                log::info!(
                    "Wrote to SSID characteristic: {:?} -> {:?}",
                    args.current_data(),
                    args.recv_data()
                );
                if let Ok(new_ssid) = String::from_utf8(args.recv_data().to_vec()) {
                    log::info!("New SSID: {}", new_ssid);
                    let mut setting = setting2.lock().unwrap();
                    if let Err(e) = setting.1.set_str("ssid", &new_ssid) {
                        log::error!("Failed to save SSID to NVS: {:?}", e);
                    } else {
                        setting.0.ssid = new_ssid;
                    }
                } else {
                    log::error!("Failed to parse new SSID from bytes.");
                }
            });

        let setting1 = setting.clone();
        let setting2 = setting.clone();
        let pass_characteristic = service
            .lock()
            .create_characteristic(PASS_ID, NimbleProperties::READ | NimbleProperties::WRITE);
        pass_characteristic
            .lock()
            .on_read(move |c, _| {
                log::info!("Read from pass characteristic");
                let setting = setting1.lock().unwrap();
                c.set_value(setting.0.pass.as_bytes());
            })
            .on_write(move |args| {
                log::info!(
                    "Wrote to pass characteristic: {:?} -> {:?}",
                    args.current_data(),
                    args.recv_data()
                );
                if let Ok(new_pass) = String::from_utf8(args.recv_data().to_vec()) {
                    log::info!("New pass: {}", new_pass);
                    let mut setting = setting2.lock().unwrap();
                    if let Err(e) = setting.1.set_str("pass", &new_pass) {
                        log::error!("Failed to save pass to NVS: {:?}", e);
                    } else {
                        setting.0.pass = new_pass;
                    }
                } else {
                    log::error!("Failed to parse new pass from bytes.");
                }
            });

        let setting = setting.clone();
        let setting_ = setting.clone();

        let server_url_characteristic = service.lock().create_characteristic(
            SERVER_URL_ID,
            NimbleProperties::READ | NimbleProperties::WRITE,
        );
        server_url_characteristic
            .lock()
            .on_read(move |c, _| {
                log::info!("Read from server URL characteristic");
                let setting = setting.lock().unwrap();
                c.set_value(setting.0.server_url.as_bytes());
            })
            .on_write(move |args| {
                log::info!(
                    "Wrote to server URL characteristic: {:?} -> {:?}",
                    args.current_data(),
                    args.recv_data()
                );
                if let Ok(mut new_server_url) = String::from_utf8(args.recv_data().to_vec()) {
                    log::info!("New server URL: {}", new_server_url);
                    if !new_server_url.ends_with("/") {
                        new_server_url.push('/');
                    }
                    let mut setting = setting_.lock().unwrap();
                    if let Err(e) = setting.1.set_str("server_url", &new_server_url) {
                        log::error!("Failed to save server URL to NVS: {:?}", e);
                    } else {
                        setting.0.server_url = new_server_url;
                    }
                } else {
                    log::error!("Failed to parse new server URL from bytes.");
                }
            });

        ble_advertising.lock().set_data(
            BLEAdvertisementData::new()
                .name(&format!("GAIA-ESP32-{}", ble_addr))
                .add_service_uuid(SERVICE_ID),
        )?;
        ble_advertising.lock().start()?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let peripherals = esp_idf_svc::hal::prelude::Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20)?;
    let partition = esp_idf_svc::nvs::EspDefaultNvsPartition::take()?;
    let nvs = esp_idf_svc::nvs::EspDefaultNvs::new(partition, "setting", true)?;

    log_heap();

    audio::audio_init();
    ui::lcd_init();

    log_heap();
    let mut ssid_buf = [0; 32];
    let ssid = nvs
        .get_str("ssid", &mut ssid_buf)
        .map_err(|e| log::error!("Failed to get ssid: {:?}", e))
        .ok()
        .flatten();

    let mut pass_buf = [0; 64];
    let pass = nvs
        .get_str("pass", &mut pass_buf)
        .map_err(|e| log::error!("Failed to get pass: {:?}", e))
        .ok()
        .flatten();

    let mut server_url = [0; 128];
    let server_url = nvs
        .get_str("server_url", &mut server_url)
        .map_err(|e| log::error!("Failed to get server_url: {:?}", e))
        .ok()
        .flatten();

    log::info!("SSID: {:?}", ssid);
    log::info!("PASS: {:?}", pass);
    log::info!("Server URL: {:?}", server_url);
    log_heap();

    let _ = ui::backgroud();

    // Configures the button
    let mut button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio0)?;
    button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

    let mut ex_button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio3)?;
    ex_button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    ex_button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::NegEdge)?;

    let b = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut gui = ui::UI::new(None).unwrap();

    let setting = Arc::new(Mutex::new((
        Setting {
            ssid: ssid.unwrap_or_default().to_string(),
            pass: pass.unwrap_or_default().to_string(),
            server_url: server_url.unwrap_or_default().to_string(),
        },
        nvs,
    )));

    log_heap();

    let need_init = {
        let setting = setting.lock().unwrap();
        setting.0.ssid.is_empty()
            || setting.0.pass.is_empty()
            || setting.0.server_url.is_empty()
            || button.is_low()
    };
    if need_init {
        bt::bt(setting.clone()).unwrap();
        log_heap();

        gui.state = "Please setup device by bt".to_string();
        gui.text = "Goto https://echokit.dev/setup/ to set up the device.\nPress K0 to continue"
            .to_string();
        gui.display_qrcode("https://echokit.dev/setup/").unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    gui.state = "Connecting to wifi...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    let _wifi = {
        let setting = setting.lock().unwrap();
        network::wifi(
            &setting.0.ssid,
            &setting.0.pass,
            peripherals.modem,
            sysloop.clone(),
        )
    };
    if _wifi.is_err() {
        gui.state = "Failed to connect to wifi".to_string();
        gui.text = "Press K0 to restart".to_string();
        gui.display_flush().unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let wifi = _wifi.unwrap();
    log_heap();

    let mac = wifi.ap_netif().get_mac().unwrap();
    let mac_str = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let bclk = peripherals.pins.gpio21;
    let din = peripherals.pins.gpio47;
    let dout = peripherals.pins.gpio14;
    let ws = peripherals.pins.gpio13;

    let (evt_tx, evt_rx) = tokio::sync::mpsc::channel(64);
    let (tx1, rx1) = tokio::sync::mpsc::unbounded_channel();

    let i2s_task = audio::i2s_task(
        peripherals.i2s0,
        bclk.into(),
        din.into(),
        dout.into(),
        ws.into(),
        (evt_tx.clone(), rx1),
    );

    gui.state = "Connecting to server...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    let server_url = {
        let setting = setting.lock().unwrap();
        format!("{}{}", setting.0.server_url, mac_str)
    };
    let server = b.block_on(ws::Server::new(server_url.clone()));
    if server.is_err() {
        gui.state = "Failed to connect to server".to_string();
        gui.text = format!("Please check your server URL: {server_url}");
        gui.display_flush().unwrap();
        b.block_on(button.wait_for_falling_edge()).unwrap();
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let server = server.unwrap();

    let ex_evt_tx = evt_tx.clone();

    let ws_task = app::main_work(server, tx1, evt_rx);

    b.spawn(async move {
        loop {
            let _ = button.wait_for_falling_edge().await;
            log::info!("Button k0 pressed {:?}", button.get_level());

            let r = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                button.wait_for_rising_edge(),
            )
            .await;
            match r {
                Ok(_) => {
                    if evt_tx
                        .send(app::Event::Event(app::Event::K0))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send K0 event");
                        break;
                    }
                }
                Err(_) => {
                    if evt_tx
                        .send(app::Event::Event(app::Event::K0_))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send K0 event");
                        break;
                    }
                }
            }
        }
    });
    b.spawn(async move {
        loop {
            let _ = ex_button.wait_for_falling_edge().await;
            let r = unsafe { esp_idf_svc::sys::hal_driver::xl9555_key_scan(0) } as u32;
            match r {
                esp_idf_svc::sys::hal_driver::KEY0_PRES => {
                    log::info!("KEY1_PRES");
                    if ex_evt_tx
                        .send(app::Event::Event(app::Event::K1))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send K1 event");
                        break;
                    }
                }
                esp_idf_svc::sys::hal_driver::KEY1_PRES => {
                    log::info!("KEY2_PRES");
                    if ex_evt_tx
                        .send(app::Event::Event(app::Event::K2))
                        .await
                        .is_err()
                    {
                        log::error!("Failed to send K2 event");
                        break;
                    }
                }
                _ => {}
            }
        }
    });
    b.spawn(i2s_task);
    b.block_on(async move {
        let r = ws_task.await;
        if let Err(e) = r {
            log::error!("Error: {:?}", e);
        } else {
            log::info!("WebSocket task finished successfully");
        }
    });
    log::error!("WebSocket task finished");
    unsafe { esp_idf_svc::sys::esp_restart() }
}

pub fn log_heap() {
    unsafe {
        use esp_idf_svc::sys::{heap_caps_get_free_size, MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM};

        log::info!(
            "Free heap size: {}",
            heap_caps_get_free_size(MALLOC_CAP_SPIRAM)
        );
        log::info!(
            "Free internal heap size: {}",
            heap_caps_get_free_size(MALLOC_CAP_INTERNAL)
        );
    }
}
