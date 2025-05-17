use tokio::sync::mpsc;
use tokio_websockets::Message;

use crate::{
    audio::{self, AudioData},
    ws::Server,
};

#[derive(Debug)]
pub enum Event {
    Event(&'static str),
    Asr(String),
    Action(String),
    AudioStart(String),
    AudioChunk(tokio_websockets::Payload),
    AudioEnd,
    RequestEnd(u16, String),
}

#[allow(dead_code)]
impl Event {
    pub const GAIA: &'static str = "gaia";
    pub const NO: &'static str = "no";
    pub const YES: &'static str = "yes";
    pub const NOISE: &'static str = "noise";
    pub const RESET: &'static str = "reset";
    pub const UNKNOWN: &'static str = "unknown";
    pub const K0: &'static str = "k0";
    pub const K1: &'static str = "k1";
    pub const K2: &'static str = "k2";
}

pub fn clear_nvs(nvs: &mut esp_idf_svc::nvs::EspDefaultNvs) -> anyhow::Result<()> {
    nvs.remove("ssid")?;
    nvs.remove("pass")?;
    nvs.remove("server_url")?;
    Ok(())
}

async fn submit_chat(
    gui: &mut crate::ui::UI,
    server: &mut Server,
    mic_tx: &mpsc::UnboundedSender<mpsc::Sender<Vec<u8>>>,
) -> anyhow::Result<usize> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);

    if let Err(e) = mic_tx.send(tx) {
        log::error!("Error sending mic tx: {:?}", e);
        gui.state = "Error on mic tx".to_string();
        gui.display_flush().unwrap();
        return Err(anyhow::anyhow!("Error sending mic tx"));
    }

    gui.state = "Listening...".to_string();
    gui.reset = false;
    gui.display_flush().unwrap();
    log::info!("Listening...");

    let mut n = 0;
    while let Some(data) = rx.recv().await {
        server
            .send(Message::binary(bytes::Bytes::from(data)))
            .await?;
        n += 1;
    }
    if n > 0 {
        server.send(Message::text("End:Normal")).await?;
        gui.state = "Wait...".to_string();
        gui.reset = false;
        gui.display_flush().unwrap();
        log::info!("Wait...");

        while let Ok(evt) = server.recv().await {
            match evt {
                Event::Asr(text) => {
                    log::info!("Received ASR: {:?}", text);
                    gui.state = "ASR".to_string();
                    gui.text = text.trim().to_string();
                    gui.display_flush().unwrap();
                    break;
                }
                _ => {}
            }
        }
    } else {
        gui.state = "IDLE".to_string();
        gui.reset = false;
        gui.display_flush().unwrap();
        log::info!("IDLE");
    }

    Ok(n)
}

async fn select_evt(evt_rx: &mut mpsc::Receiver<Event>, server: &mut Server) -> Option<Event> {
    tokio::select! {
        Some(evt) = evt_rx.recv() => {
            log::info!("Received event: {:?}", evt);
            Some(evt)
        }
        Ok(msg) = server.recv() => {
            if matches!(msg, Event::AudioChunk(..)) {
                log::info!("Received AudioChunk");
            }else{
                log::info!("Received message: {:?}", msg);
            }
            Some(msg)
        }
        else => {
            log::info!("No events");
            None
        }
    }
}

pub async fn main_work<'d>(
    mut server: Server,
    player_tx: audio::PlayerTx,
    mic_tx: mpsc::UnboundedSender<mpsc::Sender<Vec<u8>>>,
    mut evt_rx: mpsc::Receiver<Event>,
    mut nvs: esp_idf_svc::nvs::EspDefaultNvs,
) -> anyhow::Result<()> {
    let mut gui = crate::ui::UI::default();
    let mut idle = true;

    gui.state = "Connected to server".to_string();
    gui.display_flush().unwrap();

    while let Some(evt) = select_evt(&mut evt_rx, &mut server).await {
        match evt {
            Event::Event(Event::GAIA) => {
                log::info!("Received event: gaia");
                gui.state = "gaia".to_string();
                gui.display_flush().unwrap();

                let _idle = idle;
                idle = false;

                let (tx, rx) = tokio::sync::oneshot::channel();
                player_tx
                    .send(AudioData::Hello(tx))
                    .map_err(|e| anyhow::anyhow!("Error sending hello: {e:?}"))?;

                let _ = rx.await;

                if submit_chat(&mut gui, &mut server, &mic_tx).await? == 0 {
                    idle = _idle;
                }
                player_tx
                    .send(AudioData::Start)
                    .map_err(|e| anyhow::anyhow!("Error sending start: {e:?}"))?;
            }
            Event::Event(Event::RESET) if idle => {
                log::info!("Received reset");
                gui.reset = true;
                gui.display_flush().unwrap();
            }
            Event::Event(Event::YES) if gui.reset => {
                log::info!("Received yes");
                gui.display_flush().unwrap();
                nvs.remove("ssid")?;
                nvs.remove("pass")?;
                nvs.remove("server_url")?;
                unsafe { esp_idf_svc::sys::esp_restart() };
            }
            Event::Event(Event::NO) if gui.reset => {
                log::info!("Received no");
                gui.reset = false;
                gui.display_flush().unwrap();
            }
            Event::Event(evt) => {
                log::info!("Received event: {:?}", evt);

                if idle {
                    gui.state = evt.to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::Asr(text) => {
                log::info!("Received ASR: {:?}", text);
                gui.state = "ASR".to_string();
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
            }
            Event::Action(action) => {
                log::info!("Received action");
                gui.state = format!("Action: {}", action);
                gui.display_flush().unwrap();
            }
            Event::AudioStart(text) => {
                log::info!("Received audio start: {:?}", text);
                gui.state = "Speaking...".to_string();
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
            }
            Event::AudioChunk(data) => {
                log::info!("Received audio chunk");
                if let Err(e) = player_tx.send(AudioData::Chunk(data.to_vec())) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.state = "Error on audio chunk".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::AudioEnd => {
                log::info!("Received audio end");
                gui.state = "Pause".to_string();
                let (tx, rx) = tokio::sync::oneshot::channel();
                if let Err(e) = player_tx.send(AudioData::End(tx)) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.state = "Error on audio chunk".to_string();
                    gui.display_flush().unwrap();
                }
                let _ = rx.await;
                gui.display_flush().unwrap();
            }
            Event::RequestEnd(code, msg) => {
                log::info!("Received request end: {} {}", code, msg);
                if submit_chat(&mut gui, &mut server, &mic_tx).await? == 0 {
                    idle = true;
                }
            }
        }
    }

    log::info!("Main work done");

    Ok(())
}

pub async fn app_run(
    server_url: String,
    (tx, mut audio_rx): (audio::PlayerTx, audio::MicRx),
    evt_rx: mpsc::Receiver<Event>,
    nvs: esp_idf_svc::nvs::EspDefaultNvs,
) -> anyhow::Result<()> {
    // let server_url = "ws://192.168.1.28:8080/ws/2".to_string();
    let server = crate::ws::Server::new(server_url).await?;
    let (mic_tx, mut mic_rx) = mpsc::unbounded_channel::<mpsc::Sender<Vec<u8>>>();
    tokio::spawn(async move {
        loop {
            let res = tokio::select! {
                Some(audio) = audio_rx.recv() => {
                    Err(audio)
                },
                Some(mic) = mic_rx.recv() => {
                    Ok(mic)
                }
            };
            if let Ok(tx) = res {
                if let Some(data) = audio_rx.recv().await {
                    let r = tx.send(data).await;
                    if r.is_err() {
                        log::warn!("Skip audio");
                    }
                } else {
                    break;
                };
            }
        }
    });

    main_work(server, tx, mic_tx, evt_rx, nvs).await
}
