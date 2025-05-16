use embedded_graphics::prelude::{IntoStorage, RgbColor};

mod audio;
mod ui;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    let _fs = esp_idf_svc::io::vfs::MountedEventfs::mount(20).unwrap();
    audio::audio_init();
    ui::lcd_init();

    unsafe {
        esp_idf_svc::sys::hal_driver::lcd_clear(ui::ColorFormat::RED.into_storage());
        std::thread::sleep(std::time::Duration::from_millis(1000));
        esp_idf_svc::sys::hal_driver::lcd_clear(ui::ColorFormat::GREEN.into_storage());
        std::thread::sleep(std::time::Duration::from_millis(1000));
        esp_idf_svc::sys::hal_driver::lcd_clear(ui::ColorFormat::BLUE.into_storage());
        std::thread::sleep(std::time::Duration::from_millis(1000));

        let r = ui::backgroud();
        if let Err(e) = r {
            log::error!("Error: {}", e);
        } else {
            log::info!("Background animation completed successfully");
        }
    }

    log_heap();

    let r = esp_idf_svc::hal::task::block_on(audio::i2s_test());
    if let Err(e) = r {
        log::error!("Error: {}", e);
    } else {
        log::info!("I2S test completed successfully");
    }
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
