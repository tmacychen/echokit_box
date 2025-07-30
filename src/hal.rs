#[cfg(feature = "box")]
pub fn audio_init() {
    use esp_idf_svc::sys::hal_driver;
    const SAMPLE_RATE: u32 = 16000;

    unsafe {
        hal_driver::myiic_init();
        hal_driver::xl9555_init();
        hal_driver::es8311_init(SAMPLE_RATE as i32);
        hal_driver::xl9555_pin_write(hal_driver::SPK_CTRL_IO as _, 1);
        hal_driver::es8311_set_voice_volume(75); /* 设置喇叭音量，建议不超过65 */
        hal_driver::es8311_set_voice_mute(0); /* 打开DAC */
    }
}

#[cfg(feature = "boards")]
pub fn audio_init() {}
