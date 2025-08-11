use std::{env, time::Duration};

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
    if std::fs::exists(".env")? {
        dotenv()?;
    }
    let token = env::var("DISCORD_TOKEN")?;
    let chan = env::var("DISCORD_CHANNEL")?.parse::<u64>()?;
    let api = env::var("MC_API_BASE")?; // 例: http://host:8080
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
    // 人間の発言 → say xxx: msg を /chats へ
    if let serenity::FullEvent::Message {
        new_message: message,
    } = event
    {
        if message.channel_id == data.chan_id && !message.author.bot {
            println!("received message: {}", message.content);
            let formatted = format!("/say {}: {}\n", message.author.name, message.content);
            data.send_ws.send(formatted).await?;
        }
    }
    Ok(())
}

/// 初回 listener 呼び出しで WebSocket タスクを開始 ----------------------------
pub async fn init_ws(
    api: &str,
    chan_id: ChannelId,
    ctx: serenity::Context,
) -> mpsc::Sender<String> {
    let (tx, mut rx) = mpsc::channel::<String>(32);
    let ws_url = api.replace("http", "ws") + "/chats";

    spawn(async move {
        // --- 受信側判定用正規表現 (ループ外で一度だけコンパイル) ---
        let re_join = Regex::new(r#"joined the game|left the game"#).unwrap();
        let re_adv = Regex::new(r#"has made the advancement"#).unwrap();
        // 例: [08:37:28] [Server thread/INFO]: <User> Hello!
        let re_chat = Regex::new(r#": <([^>]+)> (.+)$"#).unwrap();
        let ts = Regex::new(r#"^\[[0-9]{2}:[0-9]{2}:[0-9]{2}\]"#).unwrap();

        // ------------------------------------------------------------------
        //  再接続ループ
        // ------------------------------------------------------------------
        'reconnect: loop {
            println!("Connecting to WebSocket: {}", ws_url);
            // WebSocketサーバーへ接続
            let ws_stream = match connect_async(&ws_url).await {
                Ok((stream, _)) => {
                    println!("WebSocket connected successfully.");
                    stream
                }
                Err(e) => {
                    eprintln!("WS connect error: {}. Retrying in 5 seconds...", e);
                    // 接続失敗時は5秒待ってリトライ
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue 'reconnect;
                }
            };

            let (mut write, mut read) = ws_stream.split();

            // ------------------------------------------------------------------
            //  送受信ループ
            // ------------------------------------------------------------------
            loop {
                tokio::select! {
                    // Discord -> Minecraft
                    // mpscチャネルからメッセージを受信したら、WebSocketに書き込む
                    Some(msg_to_send) = rx.recv() => {
                        if write.send(Message::Text(msg_to_send.into())).await.is_err() {
                            eprintln!("WS send error. Connection may be closed.");
                            // 書き込みに失敗したら接続が切れている可能性が高いので、
                            // 内側ループを抜けて再接続シーケンスに入る
                            break;
                        }
                    },

                    // Minecraft -> Discord
                    // WebSocketからメッセージを受信したら、内容を解析してDiscordに送信
                    Some(Ok(msg)) = read.next() => {
                        if let Message::Text(t) = msg {
                            // （1）チャット行
                            if let Some(cap) = re_chat.captures(&t) {
                                let user = &cap[1];
                                let body = &cap[2];
                                let _ = chan_id.say(&ctx.http, format!("{user}: {body}")).await;
                                continue;
                            }

                            // （2）参加/退出・実績行
                            if re_join.is_match(&t) || re_adv.is_match(&t) {
                                let start = ts.find(&t).map(|m| m.start()).unwrap_or(0);
                                let clean = &t[start..];
                                let _ = chan_id.say(&ctx.http, clean).await;
                            }
                            // それ以外は無視
                        }
                    },
                    // select! のどちらの分岐も実行されなかった場合、
                    // readストリームが終了した（＝接続が切れた）ことを意味する
                    else => {
                        eprintln!("WS connection closed.");
                        break;
                    }
                }
            }

            // 内側ループを抜けたら、接続が切れたことを意味する
            // 5秒待ってから再接続を試みる
            println!("Disconnected from WebSocket. Reconnecting in 5 seconds...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    tx
}
