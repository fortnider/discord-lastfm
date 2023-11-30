#![allow(unused_imports)]
use futures::{SinkExt, StreamExt};
use log::debug;
use log::error;
use log::info;
use log::warn;
use log::LevelFilter;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::task::Wake;
use std::time::Duration;
use std::vec;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use toml::Table;
use tungstenite::protocol::Message;

static BACKOFF_TIMER: [u64; 7] = [10, 20, 30, 60, 120, 300, 600];
#[allow(unused_mut)]
async fn last_fm_poll(lastfm_tx: Sender<String>, last_fm_user: &str, last_fm_api_key: &str) {
    let mut last_fm_url: String = format!("https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user={}&api_key={}&limit=1&format=json", last_fm_user, last_fm_api_key);
    let mut poll_time: u64 = 10;
    let status = "online"; // "online", "dnd", "idle"

    let mut lfm_response: Value;
    let mut song: Vec<String> = vec!["".to_string(), "".to_string(), "".to_string()];
    let mut status_update: String;
    let mut status_cleared = true;

    let mut lfm_backoff_count: usize = 0;

    loop {
        'lfm_query: loop {
            match reqwest::get(&last_fm_url).await {
                Ok(response) => {
                    debug!("last.fm Response Received");
                    lfm_response = response.json().await.expect("Could not parse json");

                    match lfm_response.pointer("/error") {
                        Some(error) => {
                            let error_num = error.as_u64().unwrap();
                            match error_num {
                                8 => {
                                    //"Operation failed - Most likely the backend service failed. Please try again."
                                    sleep(Duration::from_secs(
                                        *BACKOFF_TIMER.get(lfm_backoff_count).unwrap_or(&600),
                                    ))
                                    .await;
                                    error!("Could not connect to last.fm! Waiting {} seconds before retrying.",*BACKOFF_TIMER.get(lfm_backoff_count).unwrap_or(&600));
                                    lfm_backoff_count += 1;
                                }
                                6 => {
                                    // User not found
                                    error!("LastFM could not find user. Are you sure you entered your username correctly?");
                                    panic!("LastFM could not find user. Are you sure you entered your username correctly?");
                                }
                                _ => {
                                    // Some other last.fm error message. Unsure of them, will add as I find them.
                                    println!("{}", lfm_response);
                                    panic!("some shit happened what the hell");
                                }
                            }
                        }
                        None => {
                            break 'lfm_query;
                        }
                    }
                }
                Err(_) => {
                    sleep(Duration::from_secs(
                        *BACKOFF_TIMER.get(lfm_backoff_count).unwrap_or(&600),
                    ))
                    .await;
                    error!(
                        "Could not connect to last.fm! Waiting {} seconds before retrying.",
                        *BACKOFF_TIMER.get(lfm_backoff_count).unwrap_or(&600)
                    );
                    lfm_backoff_count += 1;
                }
            };
        }
        lfm_backoff_count = 0;

        let now_playing = lfm_response["recenttracks"]["track"][0]["@attr"]["nowplaying"]
            .as_str()
            .unwrap_or_default();
        // let song_name: String;
        // match lfm_response.pointer("/recenttracks/track/0/name"){
        //     Some(song) => {
        //         song_name = song.to_string();
        //     }
        //     None => {
        //
        //         println!("{}", lfm_response.to_string());
        //         panic!("fuck")
        //     }
        // }
        //
        let song_name = lfm_response
            .pointer("/recenttracks/track/0/name")
            .unwrap()
            .to_string();
        let song_artist = lfm_response
            .pointer("/recenttracks/track/0/artist/#text")
            .unwrap()
            .to_string();
        let song_album = lfm_response
            .pointer("/recenttracks/track/0/album/#text")
            .unwrap()
            .to_string();

        debug!(
            "Now playing details: 
            Name: {}
            Artist: {}
            Album: {}
        ",
            song_name, song_artist, song_album
        );

        if now_playing == "true" {
            if song_name != song[0] || song_artist != song[1] || song_album != song[2] {
                // If song name, artist, or album are different from the previous poll
                info!("Song is currently playing and different. Updating status.");
                song = vec![song_name, song_artist, song_album];

                status_update = r#"{"op": 3, "d": {"since": 0,"activities": [{"name": "NAME","type": 2,"id": "custom", "details":"DETAILS"}],"status": "STATUS","afk": false}}"#
                    .replace("STATUS", status)
                    .replace("NAME", format!("{} - {}", &song[0], &song[1]).replace('"', "").as_str())
                    .replace("DETAILS", format!("From {}", &song[2]).replace('"', "").as_str());
                lastfm_tx
                    .send(status_update.to_string())
                    .await
                    .expect("couldnt send status update");
                status_cleared = false;
            }
        } else if !status_cleared {
            info!("Song is not currently playing and status has not been cleared. Clearing status now. ");
            status_update = r#"{"op": 3, "d": {"since": 0,"activities": null,"status": "STATUS","afk": false}}"#.to_string();
            lastfm_tx
                .send(status_update.to_string())
                .await
                .expect("couldnt send status update");
            status_cleared = true;
        }
        sleep(Duration::from_secs(poll_time)).await;
    }
}

async fn discord_io(
    heartbeat_ack_tx: Sender<bool>,
    mut rx: Receiver<String>,
    mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> u64 {
    loop {
        // Main websocket send/receive loop
        tokio::select! {
            ws_in = ws_stream.next() => {
                match ws_in{
                    Some(_) => {
                        let msg_json: Value;
                        match ws_in.unwrap(){
                            Ok(msg) => {
                                let msg_str = msg.to_text().unwrap();
                                if msg_str.is_empty() {
                                    msg_json = serde_json::from_str(r#"{"op": 20 }"#).unwrap();
                                }
                                else {
                                    match serde_json::from_str(msg.to_text().unwrap()) {
                                        Ok(msg) => {
                                            msg_json = msg;
                                        },
                                        Err(err_msg) => {
                                            error!("Failed to parse discord's json! Error message: {}", err_msg);
                                            error!("Discord's message: {}", msg.to_text().unwrap());
                                            panic!()
                                        }
                                        /*
                                        Failed to parse discord's json! Error message: expected value at line 1 column 1
                                        Discord's message: Error while decoding payload.
                                        thread 'main' panicked at 'explicit panic', src/main.rs:155:45 */
                                    }
                                }
                            },
                            Err(_) => return 69,
                        };
                        let opcode = &msg_json["op"].to_string().parse::<i32>().unwrap_or(99);
                        // Extract the opcode from the json response. If there isn't a opcode, set opcode to 99
                        match opcode{
                            11 => { //11 is discord's heartbeat ack
                                heartbeat_ack_tx.send(true).await.expect("Could not send message through heartbeat_ack");
                                debug!("Recieved heartbeat ack, sent mpsc message")
                            },

                            9 => { // 9 means session has been invalidated. Discord suggests restarting the session.
                                warn!("Opcode 9 received. Returning.");
                                return 9
                            },

                            0 => { // 0 is discord receiving whatever you sent
                            },

                            7 => { // 7 means reconnect. Discord suggests reconnecting/resuming the session. In our case we just restart.
                                warn!("Discord sent opcode 7. Returning.");
                                return 7
                            },
                            20 => { // opcode 20 is us receiving an empty message
                                warn!("Received empty message");
                            }

                            _ => {
                                warn!("Received unexpected message: {:#?}", msg_json);
                                return 99
                            },
                        }

                    },
                    None => {
                        warn!("uhhh something happened to the websocket stream. returned None. IDFK wha tthis means. stopping this shit anyway.");
                        return 69
                    },

                }
            },

            ws_out = rx.recv() => {
                match ws_out{
                    Some(ws_out) => {
                        let json_out: Value = serde_json::from_str(ws_out.as_str()).unwrap();
                        match json_out["op"].as_u64().unwrap(){
                            1 => debug!("Sent Heartbeat to Discord"),
                            _ => info!("Sent: {:#?}", ws_out),
                        }
                        let _ = ws_stream.send(Message::Text(ws_out)).await;
                    },

                    None => return 56
                }


            },
        }
    }
}

async fn heartbeat(
    heartbeat_tx: Sender<String>,
    heartbeat_interval: u64,
    mut heartbeat_ack_rx: Receiver<bool>,
) {
    sleep(Duration::from_millis(heartbeat_interval)).await;
    loop {
        heartbeat_tx
            .send(r#"{"op": 1, "d": "None"}"#.to_string())
            .await
            .expect("Failed to send heartbeat to mspc channel");
        debug!("Heartbeat Sent");
        sleep(Duration::from_millis(heartbeat_interval)).await;
        if !heartbeat_ack_rx.try_recv().unwrap_or(false) {
            warn!("No heartbeat detected! Heartbeat function returning");
            return;
        };
    }
}

async fn discord_connection(
    token: &str,
) -> (
    WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    u64,
) {
    let status = "online"; // "online", "dnd", "idle"
                           // let token: String = env::var("DISCORD_TOKEN").unwrap();
    let mut backoff_count: usize = 0;
    loop {
        info!("Attempting to connect to Discord");
        let res = reqwest::Client::new()
            .get("https://discordapp.com/api/v9/users/@me")
            .header("Authorization", token)
            .header("Content-Type", "application/json")
            .send()
            .await;

        match &res {
            Ok(resp) => {
                if resp.status().to_string() == "200 OK" {
                    info!("Token Valid, attempting websocket connection");

                    let (mut ws_stream, _) =
                        connect_async("wss://gateway.discord.gg/?v=9&encoding=json")
                            .await
                            .expect("Failed ws connect");
                    let hello = &ws_stream.next().await.unwrap().unwrap().to_string();

                    let heartbeat_interval = serde_json::from_str::<Value>(hello)
                        .unwrap()
                        .pointer("/d/heartbeat_interval")
                        .unwrap()
                        .to_string()
                        .parse::<u64>()
                        .unwrap();

                    let identity = r#"{"op": 2, "d": {"token": "TOKEN", "properties": {"$os": "Windows 10", "$browser": "Google Chrome", "$device": "Windows"}, "presence": {"status": "STATUS", "afk": false}}, "s": null, "t": null}"#
                        .replace("TOKEN", token)
                        .replace("STATUS", status);

                    ws_stream
                        .send(Message::Text(identity.to_string()))
                        .await
                        .expect("couldn't send identity... le sigh");
                    info!("Successfully connected to Discord");

                    return (ws_stream, heartbeat_interval);
                } else {
                    error!("Token Invalid or other error! \n{}", resp.status());
                    error!("Token provided is: {}", &token);
                    panic!("In")
                };
            }

            Err(_) => {
                error!(
                    "Could not connect to Discord! Waiting {} seconds before retrying.",
                    *BACKOFF_TIMER.get(backoff_count).unwrap_or(&600)
                );
            }
        };

        sleep(Duration::from_secs(
            *BACKOFF_TIMER.get(backoff_count).unwrap_or(&600),
        ))
        .await;
        backoff_count += 1;
    }
}

fn get_configs() -> (String, String, String) {
    if let Some(path) = dirs::config_dir() {
        if let Ok(config_str) = fs::read_to_string(path.join("discord_lfm/config.toml")) {
            if let Ok(config_toml) = config_str.parse::<Table>() {
                if let (Some(token), Some(user), Some(api_key)) = (
                    config_toml.get("discord_token"),
                    config_toml.get("lfm_user"),
                    config_toml.get("lfm_api_key"),
                ) {
                    (token.to_string(), user.to_string(), api_key.to_string())
                } else {
                    error!("Your config file does not have the necessary configs set.");
                    panic!("Your config file does not have the necessary configs set.");
                }
            } else {
                error!("Your config file could not be parsed (Not TOML format).");
                panic!("Your config file could not be parsed (Not TOML format).");
            }
        } else {
            error!("Your config file could not be found in the default config location.");
            panic!("Your config file could not be found in the default config location");
        }
    } else {
        error!("Your config directory could not be found or could not be accessed!");
        panic!("Your config directory could not be found or could not be accessed!");
    }
}
#[allow(unused_mut)]
#[tokio::main]
async fn main() {
    simple_logging::log_to_file("log.log", LevelFilter::Info).expect("Couldn't log");

    // Read/Load Config

    let (token, lfm_user, lfm_api_key) = get_configs();
    println!(
        "Token: {} \n lfm_user: {} \n lfm_api_key: {}",
        token, lfm_user, lfm_api_key
    );

    loop {
        let (mut ws_stream, heartbeat_interval) = discord_connection(&token).await;
        let (heartbeat_tx, mut rx) = mpsc::channel::<String>(3);
        let lastfm_tx = heartbeat_tx.clone();
        let (heartbeat_ack_tx, mut heartbeat_ack_rx) = mpsc::channel::<bool>(1);
        tokio::select! {
            _ = last_fm_poll(lastfm_tx, &lfm_user, &lfm_api_key) => (),


            failcode = discord_io(heartbeat_ack_tx, rx, ws_stream) => {
                match failcode{
                    9 => {
                         //Continue
                    },
                    7 => {
                         //Continue
                    },
                    69 => {
                         //Continue
                    }
                    _ => {
                        error!("Program returned unexpected failcode {}", failcode);
                        break
                    },
                }
            },
            _ = heartbeat(heartbeat_tx, heartbeat_interval, heartbeat_ack_rx) => (),
        }
    }
    error!("Something happened, Program terminating");
}
