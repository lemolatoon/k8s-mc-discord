use std::env;

use dotenvy::dotenv;
use poise::{
    Framework, FrameworkOptions,
    serenity_prelude::{
        self as serenity,
        futures::{SinkExt as _, StreamExt as _},
    },
};
use regex::Regex;
use reqwest::Client;
use serenity::{model::prelude::*, prelude::GatewayIntents};
use tokio::{spawn, sync::mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// ------ 共通型 --------------------------------------------------------------
type Error = anyhow::Error;
type Context<'a> = poise::Context<'a, Data, Error>;

#[derive(Clone)]
struct Data {
    chan_id: ChannelId,
    http: Client,
    api: String,
    send_ws: mpsc::Sender<String>,
}

/// ------ /list コマンド ------------------------------------------------------
#[poise::command(slash_command)]
async fn list(ctx: Context<'_>) -> Result<(), Error> {
    let url = format!("{}/list", ctx.data().api);
    let txt = ctx.data().http.get(&url).send().await?.text().await?;
    println!("List response: {}", txt);
    ctx.say(txt).await?;
    Ok(())
}

/// ------ エントリポイント ----------------------------------------------------
#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenv()?;
    let token = env::var("DISCORD_TOKEN")?;
    let chan = env::var("DISCORD_CHANNEL")?.parse::<u64>()?;
    let api = env::var("MC_API_BASE")?; // http://host:8080
    let reqwest_client = Client::new();

    let intents = serenity::GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;

    // --- Poise フレームワーク -------------------------------------------------
    let framework: Framework<Data, Error> = Framework::builder()
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                let sender = init_ws(&api, ChannelId::new(chan), ctx.clone()).await;

                Ok(Data {
                    chan_id: ChannelId::new(chan),
                    http: reqwest_client,
                    api,
                    send_ws: sender,
                })
            })
        })
        .options(FrameworkOptions {
            commands: vec![list()],
            event_handler: |ctx, event, _framework, data| {
                Box::pin(handle_events(ctx, event, data.clone()))
            },
            ..Default::default()
        })
        .build();

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    client.start().await?;
    Ok(())
}

/// ------ Discord イベント処理（MessageCreate だけ使う） ----------------------
async fn handle_events(
    _ctx: &serenity::Context,
    event: &serenity::FullEvent,
    data: Data,
) -> Result<(), Error> {
    if let serenity::FullEvent::Message {
        new_message: message,
    } = event
    {
        // 人間の発言 → say xxx: msg を /chats へ
        if message.channel_id == data.chan_id && !message.author.bot {
            println!("received message: {}", message.content);
            let formatted = format!("say {}: {}", message.author.name, message.content);
            data.send_ws.send(formatted).await?;
        }
    }
    Ok(())
}

/// 初回 listener 呼び出しで WebSocket タスクを開始
async fn init_ws(api: &str, chan_id: ChannelId, ctx: serenity::Context) -> mpsc::Sender<String> {
    let (tx, mut rx) = mpsc::channel::<String>(32);
    let ws_url = api.replace("http", "ws") + "/chats";

    spawn(async move {
        println!("Connecting to WebSocket: {}", ws_url);
        let (ws_stream, _) = match connect_async(&ws_url).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("WS connect error: {e}");
                return;
            }
        };
        let (mut write, mut read) = ws_stream.split();

        // Discord→MC
        spawn(async move {
            while let Some(msg) = rx.recv().await {
                let _ = write.send(Message::Text(msg.into())).await;
            }
        });

        // MC→Discord
        let re_join = Regex::new(r#"joined the game|left the game"#).unwrap();
        let re_adv = Regex::new(r#"has made the advancement"#).unwrap();
        let ts = Regex::new(r#"^\[[0-9]{2}:[0-9]{2}:[0-9]{2}\]"#).unwrap();

        while let Some(Ok(msg)) = read.next().await {
            if let Message::Text(t) = msg {
                if !(re_join.is_match(&t) || re_adv.is_match(&t)) {
                    continue;
                }
                let start = ts.find(&t).map(|m| m.start()).unwrap_or(0);
                let clean = &t[start..];
                let _ = chan_id.say(&ctx.http, clean).await;
            }
        }
    });
    tx
}
