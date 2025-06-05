use esp_idf_svc::eventloop::EspSystemEventLoop;

mod app;
mod audio;
mod network;
mod protocol;
mod ui;
mod ws;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let peripherals = esp_idf_svc::hal::prelude::Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20)?;
    let partition = esp_idf_svc::nvs::EspDefaultNvsPartition::take()?;
    let mut nvs = esp_idf_svc::nvs::EspDefaultNvs::new(partition, "setting", true)?;

    audio::audio_init();
    ui::lcd_init();

    let mut ssid_buf = [0; 32];
    let ssid = nvs
        .get_str("ssid", &mut ssid_buf)
        .map_err(|_| anyhow::anyhow!("Failed to get ssid"))?;

    let mut pass_buf = [0; 64];
    let pass = nvs
        .get_str("pass", &mut pass_buf)
        .map_err(|_| anyhow::anyhow!("Failed to get pass"))?;

    let mut server_url = [0; 128];
    let server_url = nvs
        .get_str("server_url", &mut server_url)
        .map_err(|_| anyhow::anyhow!("Failed to get server_url"))?;

    log::info!("SSID: {:?}", ssid);
    log::info!("PASS: {:?}", pass);
    log::info!("Server URL: {:?}", server_url);

    let _ = ui::backgroud();

    // Configures the button
    let mut button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio0)?;
    button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::PosEdge)?;

    let mut ex_button = esp_idf_svc::hal::gpio::PinDriver::input(peripherals.pins.gpio3)?;
    ex_button.set_pull(esp_idf_svc::hal::gpio::Pull::Up)?;
    ex_button.set_interrupt_type(esp_idf_svc::hal::gpio::InterruptType::NegEdge)?;

    let mut gui = ui::UI::default();

    let (ssid, pass, mut server_url) = match (ssid, pass, server_url) {
        (Some(ssid), Some(pass), Some(server_url)) => {
            (ssid.to_string(), pass.to_string(), server_url.to_string())
        }
        _ => {
            gui.state = "http://192.168.71.1".to_string();
            gui.text = format!(
                "Please connect to wifi {}.\nOpen URL: http://192.168.71.1",
                network::SSID,
            );
            gui.display_flush().unwrap();

            let from_data = network::wifi_http_server(peripherals.modem, sysloop.clone())?;
            log::info!("GET SSID: {:?}", from_data.wifi_username);
            log::info!("GET PASS: {:?}", from_data.wifi_password);
            log::info!("GET Server URL: {:?}", from_data.server_url);
            nvs.set_str("ssid", &from_data.wifi_username)?;
            nvs.set_str("pass", &from_data.wifi_password)?;
            nvs.set_str("server_url", &from_data.server_url)?;

            unsafe { esp_idf_svc::sys::esp_restart() }
        }
    };

    gui.state = "Connecting to wifi...".to_string();
    gui.text.clear();
    gui.display_flush().unwrap();

    let b = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut _wifi = network::wifi(&ssid, &pass, peripherals.modem, sysloop);
    if _wifi.is_err() {
        b.block_on(async {
            for i in 0..30 {
                let i = 30 - i;
                gui.state = format!("Failed to connect to wifi [{ssid}]");
                gui.text = format!("Restart device in {i} seconds...\nIf you want to reset the device,\nyou can press K0");
                gui.display_flush().unwrap();
                let r = tokio::time::timeout(std::time::Duration::from_secs(1), button.wait_for_falling_edge()).await;
                match r {
                    Ok(evt) => {
                        if evt.is_ok(){
                            app::clear_nvs(&mut nvs).unwrap();
                            break;
                        }
                    },
                    Err(_) =>{},
                }
            }
        });
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let wifi = _wifi.unwrap();
    let mac = wifi.ap_netif().get_mac().unwrap();
    let mac_str = format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    if !server_url.ends_with("/") {
        server_url.push('/');
    }
    server_url.push_str(&mac_str);

    let bclk = peripherals.pins.gpio21;
    let din = peripherals.pins.gpio47;
    let dout = peripherals.pins.gpio14;
    let ws = peripherals.pins.gpio13;

    let (audio_dev, chan) = audio::new_audio_chan();

    let i2s_task = audio::i2s_task(
        peripherals.i2s0,
        bclk.into(),
        din.into(),
        dout.into(),
        ws.into(),
        chan,
    );

    let server = b.block_on(ws::Server::new(server_url));
    if server.is_err() {
        b.block_on(async {
            for i in 0..10 {
                let i = 10 - i;
                gui.state = format!("Failed to connect to server");
                gui.text = format!("Restart device in {i} seconds...\nIf you want to reset the device,\nyou can press K0");
                gui.display_flush().unwrap();
                let r = tokio::time::timeout(std::time::Duration::from_secs(1), button.wait_for_falling_edge()).await;
                match r {
                    Ok(evt) => {
                        if evt.is_ok(){
                            app::clear_nvs(&mut nvs).unwrap();
                            break;
                        }
                    },
                    Err(_) =>{},
                }
            }
        });
        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let server = server.unwrap();

    let (evt_tx, evt_rx) = tokio::sync::mpsc::channel(10);
    let ex_evt_tx = evt_tx.clone();

    let ws_task = app::app_run(server, audio_dev, evt_rx, nvs);

    b.spawn(async move {
        loop {
            let _ = button.wait_for_falling_edge().await;
            log::info!("Button k0 pressed {:?}", button.get_level());
            if evt_tx
                .send(app::Event::Event(app::Event::K0))
                .await
                .is_err()
            {
                log::error!("Failed to send K0 event");
                break;
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
        use esp_idf_svc::sys::{heap_caps_get_free_size, MALLOC_CAP_8BIT};

        log::info!(
            "Free heap size: {}",
            heap_caps_get_free_size(MALLOC_CAP_8BIT)
        );
    }
}
