use embedded_graphics::prelude::{IntoStorage, RgbColor};
use esp_idf_svc::eventloop::EspSystemEventLoop;

mod audio;
mod network;
mod protocol;
mod ui;

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

    let mut gui = ui::UI::default();

    let (ssid, pass, server_url) = match (ssid, pass, server_url) {
        (Some(ssid), Some(pass), Some(server_url)) => {
            (ssid.to_string(), pass.to_string(), server_url.to_string())
        }
        _ => {
            gui.state = "http://192.168.71.1".to_string();
            gui.text = format!(
                "Please connect to wifi {}.\n\nOpen URL: http://192.168.71.1",
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

    let _wifi = network::wifi(&ssid, &pass, peripherals.modem, sysloop);
    if _wifi.is_err() {
        for i in 0..3 {
            let i = 3 - i;
            gui.state = format!("Failed to connect to wifi [{ssid}]");
            gui.text = format!("Reset device in {i} seconds...");
            gui.display_flush().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        nvs.remove("ssid")?;
        nvs.remove("pass")?;
        nvs.remove("server_url")?;

        unsafe { esp_idf_svc::sys::esp_restart() }
    }

    let mut gui = ui::UI::default();
    gui.state = "Connecting".to_string();
    gui.text = "Connecting to WiFi...".to_string();
    let r = gui.display_flush();
    if let Err(e) = r {
        log::error!("Error: {}", e);
    } else {
        log::info!("Display flushed successfully");
    }

    let bclk = peripherals.pins.gpio21;
    let din = peripherals.pins.gpio47;
    let dout = peripherals.pins.gpio14;
    let ws = peripherals.pins.gpio13;

    esp_idf_svc::hal::task::block_on(audio::i2s_task(
        peripherals.i2s0,
        bclk.into(),
        din.into(),
        dout.into(),
        ws.into(),
    ));
    // if let Err(e) = r {
    //     log::error!("Error: {}", e);
    // } else {
    //     log::info!("I2S test completed successfully");
    // }
    Ok(())
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
