use esp_idf_svc::hal::{
    gpio::AnyIOPin,
    i2s::{config, I2s, I2sDriver},
    peripheral::Peripheral,
};

use esp_idf_svc::sys::esp_sr;

const SAMPLE_RATE: u32 = 16000;
const PORT_TICK_PERIOD_MS: u32 = 1000 / esp_idf_svc::sys::configTICK_RATE_HZ;

fn max(buf: &[u8]) -> (i32, i32) {
    let mut max = 0;
    let mut min = 0;
    for x in buf.chunks(4) {
        let sample = i16::from_le_bytes([x[0], x[1]]) as i32;
        max = max.max(sample.abs());
        min = min.min(sample.abs());
    }
    (max, min)
}

fn data_to_f32(buf: &[u8]) -> (f32, Vec<f32>) {
    let mut max = 0.0;
    let mut data = Vec::with_capacity(buf.len() / 2);
    for x in buf.chunks(2) {
        let sample = i16::from_le_bytes([x[0], x[1]]) as f32;
        if sample.abs() > max {
            max = sample.abs();
        }
        data.push(sample);
    }
    (max, data)
}

fn mut_data(buf: &mut [u8]) {
    for x in buf.chunks_mut(2) {
        let sample = i16::from_le_bytes([x[0], x[1]]) as i32;
        let sample = sample / 10;
        let y = sample.to_le_bytes();
        x[0] = y[0];
        x[1] = y[1];
    }
}

unsafe fn afe_init() -> (
    *mut esp_sr::esp_afe_sr_iface_t,
    *mut esp_sr::esp_afe_sr_data_t,
) {
    let models = esp_sr::esp_srmodel_init("model\0".as_ptr() as *const i8);
    let afe_config = esp_sr::afe_config_init(
        "M\0".as_ptr() as _,
        models,
        esp_sr::afe_type_t_AFE_TYPE_VC,
        esp_sr::afe_mode_t_AFE_MODE_LOW_COST,
    );
    let afe_config = afe_config.as_mut().unwrap();
    afe_config.pcm_config.total_ch_num = 1;
    afe_config.pcm_config.mic_num = 1;
    afe_config.pcm_config.ref_num = 0;
    afe_config.pcm_config.sample_rate = 16000;
    // afe_config.afe_ringbuf_size = 10;
    afe_config.vad_mode = esp_sr::vad_mode_t_VAD_MODE_0;

    log::info!("{afe_config:?}");

    let afe_ringbuf_size = afe_config.afe_ringbuf_size;
    log::info!("afe ringbuf size: {}", afe_ringbuf_size);

    let afe_handle = esp_sr::esp_afe_handle_from_config(afe_config);
    let afe_handle = afe_handle.as_mut().unwrap();
    let afe_data = (afe_handle.create_from_config.unwrap())(afe_config);
    let audio_chunksize = (afe_handle.get_feed_chunksize.unwrap())(afe_data);
    log::info!("audio chunksize: {}", audio_chunksize);

    esp_sr::afe_config_free(afe_config);
    (afe_handle, afe_data)
}

pub static WAKE_WAV: &[u8] = include_bytes!("../assets/hello.wav");

pub async fn i2s_test() -> anyhow::Result<()> {
    let (afe_handle, afe_data) = unsafe { afe_init() };

    let timer = esp_idf_svc::timer::EspTimerService::new()?;
    log::info!("PORT_TICK_PERIOD_MS = {}", PORT_TICK_PERIOD_MS);
    let peripherals = esp_idf_svc::hal::peripherals::Peripherals::take().unwrap();
    let i2s_config = config::StdConfig::new(
        config::Config::default().auto_clear(true),
        config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE),
        config::StdSlotConfig::philips_slot_default(
            config::DataBitWidth::Bits16,
            config::SlotMode::Mono,
        ),
        config::StdGpioConfig::default(),
    );

    let bclk = peripherals.pins.gpio21;
    let din = peripherals.pins.gpio47;
    let dout = peripherals.pins.gpio14;
    let ws = peripherals.pins.gpio13;

    let mclk: Option<esp_idf_svc::hal::gpio::AnyIOPin> = None;

    let mut driver =
        I2sDriver::new_std_bidir(peripherals.i2s0, &i2s_config, bclk, din, dout, mclk, ws).unwrap();
    driver.tx_enable()?;
    driver.rx_enable()?;

    let mut buf = vec![0u8; 5 * 2 * SAMPLE_RATE as usize];
    let mut send_buf = vec![];
    let n = driver.read(&mut buf, 1000 / PORT_TICK_PERIOD_MS)?;

    driver.write_all_async(&WAKE_WAV).await?;

    for i in 0..1 {
        log::info!("Iteration {}", i);
        log::info!("Reading...");

        let n = driver.read(&mut buf, 1000 / PORT_TICK_PERIOD_MS)?;
        log::info!("Read {} bytes", n);
        log::info!("Max: {:?}", max(&buf[..n]));
        // mut_data(&mut buf[..n]);

        for i2s_buff in buf[..n].chunks(160 * 2) {
            unsafe {
                let n = (afe_handle.as_mut().unwrap().feed.unwrap())(
                    afe_data,
                    i2s_buff.as_ptr() as *const i16,
                );
                log::info!("Feed: {}", n);
                if n == 0 {
                    loop {
                        let result = (afe_handle.as_mut().unwrap().fetch.unwrap())(afe_data);
                        let result = result.as_mut().unwrap();
                        let data_size = result.data_size;
                        log::info!("Result: {}", data_size);
                        let vad_state = result.vad_state;
                        log::info!("VAD state: {}", vad_state);
                        if vad_state == esp_sr::vad_state_t_VAD_SPEECH {
                            let data_ptr: *const i16 = result.data;
                            let data = std::slice::from_raw_parts(
                                data_ptr as *const u8,
                                (data_size) as usize,
                            );
                            send_buf.extend_from_slice(data);
                        }

                        if data_size == 0 {
                            break;
                        }
                    }
                    let n = (afe_handle.as_mut().unwrap().feed.unwrap())(
                        afe_data,
                        i2s_buff.as_ptr() as *const i16,
                    );
                }
            }
        }

        log::info!("Writing...");
        driver.write_all_async(&send_buf).await?;
        log::info!("Wrote {} bytes", send_buf.len());
        // driver.tx_disable()?;
        timer
            .timer_async()?
            .after(std::time::Duration::from_secs(1))
            .await?;
    }

    Ok(())
}
