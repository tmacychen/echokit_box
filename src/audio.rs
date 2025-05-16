use std::sync::Arc;

use esp_idf_svc::hal::i2s::{config, I2sDriver, I2sRx};

use esp_idf_svc::sys::esp_sr;

const SAMPLE_RATE: u32 = 16000;
const PORT_TICK_PERIOD_MS: u32 = 1000 / esp_idf_svc::sys::configTICK_RATE_HZ;

pub fn audio_init() {
    use esp_idf_svc::sys::hal_driver;
    unsafe {
        hal_driver::myiic_init();
        hal_driver::xl9555_init();
        hal_driver::es8311_init(SAMPLE_RATE as i32);
        hal_driver::xl9555_pin_write(hal_driver::SPK_CTRL_IO as _, 1);
        hal_driver::es8311_set_voice_volume(65); /* 设置喇叭音量，建议不超过65 */
        hal_driver::es8311_set_voice_mute(0); /* 打开DAC */
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
    afe_config.afe_ringbuf_size = 25;
    afe_config.vad_mode = esp_sr::vad_mode_t_VAD_MODE_4;
    // afe_config.agc_init = true;

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

struct AFE {
    handle: *mut esp_sr::esp_afe_sr_iface_t,
    data: *mut esp_sr::esp_afe_sr_data_t,
    feed_chunksize: usize,
}

unsafe impl Send for AFE {}
unsafe impl Sync for AFE {}

struct AFEResult {
    data: Vec<u8>,
    speech: bool,
}

impl AFE {
    fn new() -> Self {
        unsafe {
            let (handle, data) = afe_init();
            let feed_chunksize =
                (handle.as_mut().unwrap().get_feed_chunksize.unwrap())(data) as usize;

            AFE {
                handle,
                data,
                feed_chunksize,
            }
        }
    }
    // returns the number of bytes fed

    fn reset(&self) {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            (afe_handle.as_ref().unwrap().reset_vad.unwrap())(afe_data);
        }
    }

    fn feed(&self, data: &[u8]) -> i32 {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            (afe_handle.as_ref().unwrap().feed.unwrap())(afe_data, data.as_ptr() as *const i16)
        }
    }

    fn fetch(&self) -> Result<AFEResult, i32> {
        let afe_handle = self.handle;
        let afe_data = self.data;
        unsafe {
            let result = (afe_handle.as_ref().unwrap().fetch.unwrap())(afe_data)
                .as_mut()
                .unwrap();

            if result.ret_value != 0 {
                return Err(result.ret_value);
            }

            let data_size = result.data_size;
            let vad_state = result.vad_state;
            let mut data = Vec::with_capacity(data_size as usize + result.vad_cache_size as usize);
            if result.vad_cache_size > 0 {
                let data_ptr = result.vad_cache as *const u8;
                let data_ = std::slice::from_raw_parts(data_ptr, (result.vad_cache_size) as usize);
                data.extend_from_slice(data_);
            }
            if data_size > 0 {
                let data_ptr = result.data as *const u8;
                let data_ = std::slice::from_raw_parts(data_ptr, (data_size) as usize);
                data.extend_from_slice(data_);
            };

            let speech = vad_state == esp_sr::vad_state_t_VAD_SPEECH;
            Ok(AFEResult { data, speech })
        }
    }
}

pub static WAKE_WAV: &[u8] = include_bytes!("../assets/hello.wav");

struct DriverI2sRx<'d>(esp_idf_svc::hal::i2s::I2sDriverRef<'d, I2sRx>);
struct DriverI2sTx<'d>(esp_idf_svc::hal::i2s::I2sDriverRef<'d, I2sRx>);

unsafe impl Send for DriverI2sRx<'_> {}
unsafe impl Send for DriverI2sTx<'_> {}

pub async fn i2s_task() {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let afe_handle = Arc::new(AFE::new());
    let afe_handle_ = afe_handle.clone();
    let afe_r = std::thread::spawn(|| afe_worker(afe_handle_, tx));
    let r = i2s_test_1(afe_handle, rx).await;
    if let Err(e) = r {
        log::error!("Error: {}", e);
    } else {
        log::info!("I2S test completed successfully");
    }
    let r = afe_r.join().unwrap();
    if let Err(e) = r {
        log::error!("Error: {}", e);
    } else {
        log::info!("AFE worker completed successfully");
    }
}

fn afe_worker(
    afe_handle: Arc<AFE>,
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) -> anyhow::Result<()> {
    let mut speech = false;
    let mut send_buf = vec![];
    loop {
        let result = afe_handle.fetch();
        if let Err(e) = &result {
            continue;
        }
        let result = result.unwrap();
        // log::info!("Result: {}", result.data.len());
        // log::info!("VAD state: {}", vad_state);
        // log::info!("VAD cache: {:?}", result.vad_cache_size);
        if result.data.is_empty() {
            break;
        }

        if result.speech {
            speech = true;
            send_buf.extend_from_slice(&result.data);
            // if send_buf.len() > SAMPLE_RATE as usize / 10 {
            //     log::info!("Sending {} bytes", send_buf.len());
            //     tx.send(send_buf)
            //         .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            //     send_buf = vec![]
            // }
            continue;
        }

        if speech {
            log::info!("Sending {} bytes", send_buf.len());
            tx.send(send_buf)
                .map_err(|_| anyhow::anyhow!("Failed to send data"))?;
            send_buf = vec![];
            speech = false;
        }
    }
    Ok(())
}

async fn i2s_test_1(
    afe_handle: Arc<AFE>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
) -> anyhow::Result<()> {
    // let timer = esp_idf_svc::timer::EspTimerService::new()?;
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

    // let (mut rx, mut tx) = driver.split();

    let mut buf = [0u8; 2 * 160];

    driver.write_all_async(&WAKE_WAV).await?;

    loop {
        match rx.try_recv() {
            Ok(data) => {
                log::info!("Send {} bytes", data.len());
                driver.write_all_async(&data).await?;
                afe_handle.reset();
            }
            _ => {
                let n = driver.read(&mut buf, 100 / PORT_TICK_PERIOD_MS)?;
                let n = afe_handle.feed(&buf[..n]);
            }
        }
    }

    // Ok(())
}

pub async fn i2s_test() -> anyhow::Result<()> {
    // let (afe_handle, afe_data) = unsafe { afe_init() };

    // let timer = esp_idf_svc::timer::EspTimerService::new()?;
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

    let afe_handle = AFE::new();

    let mut driver =
        I2sDriver::new_std_bidir(peripherals.i2s0, &i2s_config, bclk, din, dout, mclk, ws).unwrap();
    driver.tx_enable()?;
    driver.rx_enable()?;

    // let (mut rx, mut tx) = driver.split();

    let mut buf = [0u8; 2 * 160];
    let mut send_buf = vec![];

    driver.write_all_async(&WAKE_WAV).await?;

    let mut speech = false;

    'a: loop {
        let n = driver.read_async(&mut buf).await?;
        log::info!("Read {} bytes", n);

        let i2s_chunk = &buf[..n];
        {
            let n = afe_handle.feed(i2s_chunk);
            log::info!("Feed: {}", n);
            if n == 0 {
                loop {
                    let result = afe_handle.fetch();
                    if let Err(e) = &result {
                        log::error!("Error fetching: {}", *e);
                        break;
                    }
                    let result = result.unwrap();
                    log::info!("Result: {}", result.data.len());
                    // log::info!("VAD state: {}", vad_state);
                    // log::info!("VAD cache: {:?}", result.vad_cache_size);
                    if result.data.is_empty() {
                        break;
                    }

                    if result.speech {
                        speech = true;
                        send_buf.extend_from_slice(&result.data);
                        continue;
                    }

                    if speech {
                        log::info!("Sending {} bytes", send_buf.len());
                        driver.write_all_async(&send_buf).await?;
                        send_buf.clear();
                        speech = false;
                        // skip = 10;
                        continue 'a;
                    }
                }
                let n = afe_handle.feed(i2s_chunk);
            }
        }
    }

    // Ok(())
}
