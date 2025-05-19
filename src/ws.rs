#[allow(unused)]
fn print_stack_high() {
    let stack_high =
        unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark2(std::ptr::null_mut()) };
    log::info!("Stack high: {}", stack_high);
}

use crate::{app::Event, protocol::JsonCommand};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio_websockets::Message;

pub struct Server {
    pub uri: String,
    timeout: std::time::Duration,
    ws: tokio_websockets::WebSocketStream<tokio_websockets::MaybeTlsStream<tokio::net::TcpStream>>,
}

impl Server {
    pub async fn new(uri: String) -> anyhow::Result<Self> {
        let (ws, _resp) = tokio_websockets::ClientBuilder::new()
            .uri(&uri)?
            .connect()
            .await?;

        let timeout = std::time::Duration::from_secs(30);

        Ok(Self { uri, timeout, ws })
    }

    pub fn set_timeout(&mut self, timeout: std::time::Duration) {
        self.timeout = timeout;
    }

    pub async fn send(&mut self, msg: Message) -> anyhow::Result<()> {
        tokio::time::timeout(self.timeout, self.ws.send(msg))
            .map_err(|_| anyhow::anyhow!("Timeout sending message"))
            .await??;
        Ok(())
    }

    pub async fn recv(&mut self) -> anyhow::Result<Event> {
        let msg = self
            .ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("WS channel closed"))??;
        if let Some(text) = msg.as_text() {
            if let Ok(cmd) = serde_json::from_str::<JsonCommand>(text) {
                match cmd {
                    JsonCommand::Action { action } => Ok(Event::Action(action)),
                    JsonCommand::StartAudio { text } => Ok(Event::AudioStart(text)),
                    JsonCommand::EndAudio => Ok(Event::AudioEnd),
                    JsonCommand::ASR { text } => Ok(Event::Asr(text)),
                    JsonCommand::EndResponse => Ok(Event::RequestEnd(0, String::new())),
                    _ => Err(anyhow::anyhow!("Invalid command: {:?}", text)),
                }
            } else {
                log::warn!("Invalid command: {:?}", text);
                Err(anyhow::anyhow!("Invalid command: {:?}", text))
            }
        } else if msg.is_binary() {
            let payload = msg.into_payload();
            log::info!("Received binary data");
            Ok(Event::AudioChunk(payload))
        } else {
            Err(anyhow::anyhow!("Invalid message type"))
        }
    }
}
