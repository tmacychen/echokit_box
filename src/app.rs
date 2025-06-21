use tokio::sync::mpsc;
use tokio_websockets::Message;

use crate::{
    audio::{self, AudioData},
    protocol::ServerEvent,
    ws::Server,
};

#[derive(Debug)]
pub enum Event {
    Event(&'static str),
    ServerEvent(ServerEvent),
    MicAudioChunk(Vec<u8>),
    MicAudioEnd,
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

async fn select_evt(evt_rx: &mut mpsc::Receiver<Event>, server: &mut Server) -> Option<Event> {
    tokio::select! {
        Some(evt) = evt_rx.recv() => {
            match &evt {
                Event::Event(_)=>{
                    log::info!("Received event: {:?}", evt);
                },
                Event::MicAudioEnd=>{
                    log::info!("Received MicAudioEnd");
                },
                Event::MicAudioChunk(data)=>{
                    log::info!("Received MicAudioChunk with {} bytes", data.len());
                },
                Event::ServerEvent(_)=>{
                    log::info!("Received ServerEvent: {:?}", evt);
                },
            }
            Some(evt)
        }
        Ok(msg) = server.recv() => {
            match msg {
                Event::ServerEvent(ServerEvent::AudioChunk { .. })=>{
                    log::info!("Received AudioChunk");
                }
                Event::ServerEvent(ServerEvent::HelloChunk { .. })=>{
                    log::info!("Received HelloChunk");
                }
                Event::ServerEvent(ServerEvent::BGChunk { .. })=>{
                    log::info!("Received BGChunk");
                }
                _=> {
                    log::info!("Received message: {:?}", msg);
                }
            }
            Some(msg)
        }
        else => {
            log::info!("No events");
            None
        }
    }
}

// TODO: 按键打断
// TODO: 超时不监听
pub async fn main_work<'d>(
    mut server: Server,
    player_tx: audio::PlayerTx,
    mut evt_rx: mpsc::Receiver<Event>,
) -> anyhow::Result<()> {
    #[derive(PartialEq, Eq)]
    enum State {
        Listening,
        Wait,
        Speaking,
        Idle,
    }

    let mut gui = crate::ui::UI::default();

    gui.state = "Idle".to_string();
    gui.display_flush().unwrap();

    let mut new_gui_bg = vec![];

    let mut state = State::Idle;

    let mut submit_audio = 0.0;

    while let Some(evt) = select_evt(&mut evt_rx, &mut server).await {
        match evt {
            Event::Event(Event::GAIA | Event::K0) => {
                log::info!("Received event: gaia");
                // gui.state = "gaia".to_string();
                // gui.display_flush().unwrap();

                if state == State::Listening {
                    state = State::Idle;
                    gui.state = "Idle".to_string();
                    gui.display_flush().unwrap();
                } else {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    player_tx
                        .send(AudioData::Hello(tx))
                        .map_err(|e| anyhow::anyhow!("Error sending hello: {e:?}"))?;

                    let _ = rx.await;

                    state = State::Listening;
                    gui.state = "Listening...".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::Event(Event::RESET | Event::K2) => {}
            Event::Event(Event::YES | Event::K1) => {}
            Event::Event(Event::NO) => {}
            Event::Event(evt) => {
                log::info!("Received event: {:?}", evt);
            }
            Event::MicAudioChunk(data) => {
                if state == State::Listening {
                    submit_audio += data.len() as f32 / 32000.0;
                    server
                        .send(Message::binary(bytes::Bytes::from(data)))
                        .await?;
                } else {
                    log::warn!("Received MicAudioChunk while not listening");
                }
            }
            Event::MicAudioEnd => {
                if state == State::Listening && submit_audio > 1.0 {
                    server.send(Message::text("End:Normal")).await?;
                }
                submit_audio = 0.0;
            }
            Event::ServerEvent(ServerEvent::ASR { text }) => {
                log::info!("Received ASR: {:?}", text);
                gui.state = "ASR".to_string();
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
            }
            Event::ServerEvent(ServerEvent::Action { action }) => {
                log::info!("Received action");
                gui.state = format!("Action: {}", action);
                gui.display_flush().unwrap();
            }
            Event::ServerEvent(ServerEvent::StartAudio { text }) => {
                log::info!("Received audio start: {:?}", text);
                state = State::Speaking;
                gui.state = "Speaking...".to_string();
                gui.text = text.trim().to_string();
                gui.display_flush().unwrap();
                player_tx
                    .send(AudioData::Start)
                    .map_err(|e| anyhow::anyhow!("Error sending start: {e:?}"))?;
            }
            Event::ServerEvent(ServerEvent::AudioChunk { data }) => {
                log::info!("Received audio chunk");
                if state != State::Speaking {
                    log::warn!("Received audio chunk while not speaking");
                    continue;
                }

                if let Err(e) = player_tx.send(AudioData::Chunk(data)) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.state = "Error on audio chunk".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::ServerEvent(ServerEvent::EndAudio) => {
                log::info!("Received audio end");
                let (tx, rx) = tokio::sync::oneshot::channel();
                if let Err(e) = player_tx.send(AudioData::End(tx)) {
                    log::error!("Error sending audio chunk: {:?}", e);
                    gui.state = "Error on audio chunk".to_string();
                    gui.display_flush().unwrap();
                }
                let _ = rx.await;
                gui.display_flush().unwrap();
            }

            Event::ServerEvent(ServerEvent::EndResponse) => {
                log::info!("Received request end");
                state = State::Listening;
                gui.state = "Listening...".to_string();
                gui.display_flush().unwrap();
            }
            Event::ServerEvent(ServerEvent::HelloStart) => {
                if let Err(_) = player_tx.send(AudioData::SetHelloStart) {
                    log::error!("Error sending hello start");
                    gui.state = "Error on hello start".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::ServerEvent(ServerEvent::HelloChunk { data }) => {
                log::info!("Received hello chunk");
                if let Err(_) = player_tx.send(AudioData::SetHelloChunk(data.to_vec())) {
                    log::error!("Error sending hello chunk");
                    gui.state = "Error on hello chunk".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::ServerEvent(ServerEvent::HelloEnd) => {
                log::info!("Received hello end");
                if let Err(_) = player_tx.send(AudioData::SetHelloEnd) {
                    log::error!("Error sending hello end");
                    gui.state = "Error on hello end".to_string();
                    gui.display_flush().unwrap();
                } else {
                    gui.state = "Hello set".to_string();
                    gui.display_flush().unwrap();
                }
            }
            Event::ServerEvent(ServerEvent::BGStart) => {
                new_gui_bg = vec![];
            }
            Event::ServerEvent(ServerEvent::BGChunk { data }) => {
                log::info!("Received background chunk");
                new_gui_bg.extend(data);
            }
            Event::ServerEvent(ServerEvent::BGEnd) => {
                log::info!("Received background end");
                if !new_gui_bg.is_empty() {
                    let gui_ = crate::ui::UI::new(Some(&new_gui_bg));
                    new_gui_bg.clear();
                    match gui_ {
                        Ok(new_gui) => {
                            gui = new_gui;
                            gui.state = "Background data loaded".to_string();
                            gui.display_flush().unwrap();
                        }
                        Err(e) => {
                            log::error!("Error creating GUI from background data: {:?}", e);
                            gui.state = "Error on background data".to_string();
                            gui.display_flush().unwrap();
                        }
                    }
                } else {
                    log::warn!("Received empty background data");
                }
            }
            Event::ServerEvent(ServerEvent::StartVideo | ServerEvent::EndVideo) => {}
        }
    }

    log::info!("Main work done");

    Ok(())
}
