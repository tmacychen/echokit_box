use embedded_graphics::prelude::{IntoStorage, RgbColor};

mod audio;
mod ui;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20).unwrap();
    audio::audio_init();
    ui::lcd_init();

    let _ = ui::backgroud();

    let mut gui = ui::UI::default();
    gui.state = "Connecting".to_string();
    gui.text = "Connecting to WiFi...".to_string();
    let r = gui.display_flush();
    if let Err(e) = r {
        log::error!("Error: {}", e);
    } else {
        log::info!("Display flushed successfully");
    }

    log_heap();

    let r = esp_idf_svc::hal::task::block_on(audio::i2s_task());
    // if let Err(e) = r {
    //     log::error!("Error: {}", e);
    // } else {
    //     log::info!("I2S test completed successfully");
    // }
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
