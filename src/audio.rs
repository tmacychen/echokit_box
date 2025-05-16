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
    afe_config.agc_init = true;

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
            let result = (afe_handle.as_ref().unwrap().fetch_with_delay.unwrap())(afe_data, 1)
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

pub async fn i2s_worker() -> anyhow::Result<()> {
    let afe_handle = AFE::new();

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

    let (rx, tx) = driver.split();
    let mut rx = DriverI2sRx(rx);
    std::thread::scope(|s| {
        s.spawn(|| {
            i2s_rx_task(&mut rx, afe_handle).unwrap();
        });
    });

    Ok(())
}

fn i2s_rx_task<'d>(rx: &mut DriverI2sRx, afe_handle: AFE) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 5 * 2 * SAMPLE_RATE as usize];
    let mut send_buf = vec![];

    loop {
        log::info!("Reading...");
        let n = rx.0.read(&mut buf, 1000 / PORT_TICK_PERIOD_MS)?;
        log::info!("Read {} bytes", n);
        log::info!("Max: {:?}", max(&buf[..n]));

        for (j, i2s_buff) in buf[..n].chunks(160 * 2).enumerate() {
            let n = afe_handle.feed(i2s_buff);
            if n == 0 {
                log::info!("Fetching... {j}");
                let mut k = 0;
                loop {
                    let result = afe_handle.fetch();
                    if let Err(e) = &result {
                        log::error!("Error fetching: {}", *e);
                        break;
                    }
                    let result = result.unwrap();
                    log::info!("Result {k}: {}", result.data.len());
                    if result.data.is_empty() {
                        break;
                    }

                    if result.speech {
                        send_buf.extend_from_slice(&result.data);
                    }

                    k += 1;
                }
            }
        }
    }
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

    let mut buf = [0u8; 2 * 1600];
    let mut send_buf = vec![];

    driver.write_all_async(&WAKE_WAV).await?;

    let mut speech = false;

    let mut skip = 0;

    'a: loop {
        let n = driver.read(&mut buf, 1000 / PORT_TICK_PERIOD_MS)?;
        log::info!("Read {} bytes", n);
        log::info!("Max: {:?}", max(&buf[..n]));
        // if skip > 0 {
        //     skip -= 1;
        //     continue 'a;
        // }
        // mut_data(&mut buf[..n]);
        let i2s_data = &buf[..n];
        for i2s_chunk in i2s_data.chunks(160 * 2) {
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
