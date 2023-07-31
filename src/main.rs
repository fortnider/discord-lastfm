use std::time::Duration;
use std::env;
use std::vec;
use log::info;
use reqwest;
use dotenv::dotenv;
use tokio::net::TcpStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tungstenite::protocol::Message;
use tungstenite;
use tokio::sync::mpsc;
use futures::{SinkExt,StreamExt};
use serde_json::Value;
use tokio::time::sleep;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::Receiver;
use log::LevelFilter;
use log::debug;
use log::warn;
use log::error;
use std::fs;




#[allow(unused_mut)]
async fn last_fm_poll(lastfm_tx: Sender<String>, backoff_timer: &Vec<u64>){

    dotenv().ok(); 
    // Read environment variables
    let mut last_fm_api_key: String = env::var("LAST_FM_API_KEY").unwrap();
    let mut last_fm_user:String = env::var("LAST_FM_USER").unwrap();
    let mut last_fm_url: String = format!("https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user={}&api_key={}&limit=1&format=json", last_fm_user, last_fm_api_key);
    let mut poll_time: u64 = 10;
    let status = "online"; // "online", "dnd", "idle"

    let mut lfm_response:Value;
    let mut song: Vec<String> = vec!["".to_string(),"".to_string(),"".to_string()];
    let mut status_update:String;
    let mut status_cleared = true;

    let mut lfm_backoff_count: usize = 0;
    info!("fdsjklsdfjlj");
    loop{
        'lfm_query: loop{
            match reqwest::get(&last_fm_url).await{
                Ok(response) => {
                    info!("last.fm Response Received");
                    lfm_response = response.json().await.expect("Could not parse json");

                    match lfm_response.pointer("/error"){
                        Some(error) => {
                            let error_num = error.as_u64().unwrap();
                            match error_num {
                                8 => { //"Operation failed - Most likely the backend service failed. Please try again."
                                    sleep(Duration::from_secs(*backoff_timer.get(lfm_backoff_count).unwrap_or(&600))).await;
                                    error!("Could not connect to last.fm! Waiting {} seconds before retrying.",*backoff_timer.get(lfm_backoff_count).unwrap_or(&600));
                                    lfm_backoff_count += 1;
                                },
                                _ => { // Some other last.fm error message. Unsure of them, will add as I find them.
                                    println!("{}", lfm_response.to_string());
                                    panic!("FUCK");
                                }
                            }
                        },
                        None => {
                            break 'lfm_query;
                        }
                        
                    }


                },
                Err(_) => {
                    sleep(Duration::from_secs(*backoff_timer.get(lfm_backoff_count).unwrap_or(&600))).await;
                    error!("Could not connect to last.fm! Waiting {} seconds before retrying.",*backoff_timer.get(lfm_backoff_count).unwrap_or(&600));
                    lfm_backoff_count += 1;
                }
            };
        }
        lfm_backoff_count = 0;

        let now_playing = lfm_response["recenttracks"]["track"][0]["@attr"]["nowplaying"].as_str().unwrap_or_default();
        let song_name: String;
        match lfm_response.pointer("/recenttracks/track/0/name"){
            Some(song) => {
                song_name = song.to_string();
            }
            None => { 

                println!("{}", lfm_response.to_string());
                panic!("fuck")
            }
        }


        let song_artist = lfm_response.pointer("/recenttracks/track/0/artist/#text").unwrap().to_string();
        let song_album = lfm_response.pointer("/recenttracks/track/0/album/#text").unwrap().to_string();

        if now_playing == "true"
        {
            if song_name != song[0] || song_artist != song[1] || song_album != song[2]{
                info!("Song is currently playing and different. Updating status.");
                song = vec![song_name, song_artist, song_album];
    

                status_update = r#"{"op": 3, "d": {"since": 0,"activities": [{"name": "NAME","type": 2,"id": "custom", "details":"DETAILS"}],"status": "STATUS","afk": false}}"#
                    .replace("STATUS", status)
                    .replace("NAME", format!("{} - {}", &song[0], &song[1]).replace(r#"""#, "").as_str())
                    .replace("DETAILS", format!("From {}", &song[2]).replace(r#"""#, "").as_str());
                lastfm_tx.send(status_update.to_string()).await.expect("couldnt send status update"); 
                status_cleared = false;
            }
            
            
        }
        else if status_cleared == false
            {
                info!("Song is not currently playing and status has not been cleared. Clearing status now. ");
                status_update = r#"{"op": 3, "d": {"since": 0,"activities": null,"status": "STATUS","afk": false}}"#.to_string();
                lastfm_tx.send(status_update.to_string()).await.expect("couldnt send status update"); 
                status_cleared = true;

            }
    sleep(Duration::from_secs(poll_time)).await;

    }
}


async fn discord_io(heartbeat_ack_tx: Sender<bool>, mut rx: Receiver<String>, mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>) -> u64{

    loop{ // Main websocket send/receive loop
        tokio::select!{
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
                                            println!("Failed to parse discord's json! Error message: {}", err_msg);
                                            println!("Discord's message: {}", msg.to_text().unwrap());
                                            panic!()
                                        }
                                    }
                                }
                                // thread 'main' panicked at 'called `Result::unwrap()` on an `Err` value: Error("expected value", line: 1, column: 1)', src/main.rs:128:93
//                                match msg_json = serde_json::from_str(msg.to_text().unwrap()).unwrap();
                            },
                            Err(_) => return 69,
                        };
                        let opcode = &msg_json["op"].to_string().parse::<i32>().unwrap_or(99);
                        // Extract the opcode from the json response. If there isn't a opcode, set opcode to 99
                        match opcode{
                            11 => { //11 is discord's heartbeat ack
                                heartbeat_ack_tx.send(true).await.expect("Could not send message through heartbeat_ack");
                                info!("Recieved heartbeat ack, sent mpsc message")
                            }, 

                            9 => {
                                warn!("Opcode 9 received. Returning.");
                                return 9
                            },

                            0 => { // 0 is discord receiving whatever you sent
//                              println!("discord received")
                            },

                            7 => {
                                warn!("Discord sent opcode 7. Returning.");
                                return 7
                            },
                            20 => { // opcode 20 is us receiving an empty message
                                info!("Received empty message");
                            }

                            _ => {
                                info!("Received unexpected message: {:#?}", msg_json);
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
                        info!("Sent: {:#?}", ws_out);
                        let _ = ws_stream.send(Message::Text(ws_out)).await;
                    },

                    None => return 56
                }


            },
        }

    }
}

async fn heartbeat(heartbeat_tx: Sender<String>, heartbeat_interval: u64, mut heartbeat_ack_rx: Receiver<bool>){
    sleep(Duration::from_millis(heartbeat_interval)).await;
    loop{
        heartbeat_tx.send(r#"{"op": 1, "d": "None"}"#.to_string()).await.expect("Failed to send heartbeat to mspc channel");
        debug!("Heartbeat Sent");
        sleep(Duration::from_millis(heartbeat_interval)).await;
        if heartbeat_ack_rx.try_recv().unwrap_or(false) == false {
            warn!("No heartbeat detected! Heartbeat function returning");
            return

        };
    }
}

async fn discord_connection(backoff_timer: &Vec<u64>) -> (WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, u64){
    let status = "online"; // "online", "dnd", "idle"
    let token:String = env::var("DISCORD_TOKEN").unwrap();
    let mut backoff_count: usize = 0;

    loop{
        info!("Attempting to connect to Discord");
        let res = reqwest::Client::new()
        .get("https://discordapp.com/api/v9/users/@me")
        .header("Authorization", &token)
        .send()
        .await;
        
        match &res {
            Ok(resp) => {
                if resp.status().to_string() == "200 OK"{
                    info!("Token Valid, attempting websocket connection");

                    let (mut ws_stream, _) = connect_async("wss://gateway.discord.gg/?v=9&encoding=json").await.expect("Failed ws connect");
                    let hello = &ws_stream.next().await.unwrap().unwrap().to_string();

                    let heartbeat_interval = serde_json::from_str::<Value>(hello).unwrap().pointer("/d/heartbeat_interval").unwrap().to_string().parse::<u64>().unwrap();
            
                    let identity = r#"{"op": 2, "d": {"token": "TOKEN", "properties": {"$os": "Windows 10", "$browser": "Google Chrome", "$device": "Windows"}, "presence": {"status": "STATUS", "afk": false}}, "s": null, "t": null}"#
                        .replace("TOKEN", &token)
                        .replace("STATUS", &status);
            
                    ws_stream.send(Message::Text(identity.to_string())).await.expect("couldn't send identity... le sigh");
                    info!("Successfully connected to Discord");


                    return (ws_stream, heartbeat_interval)
                    }
                    else {
                        error!("Token Invalid or other error! \n{}", resp.status());
                    
                };
                
            },

            Err(_) => {
                error!("Could not connect to Discord! Waiting {} seconds before retrying.",*backoff_timer.get(backoff_count).unwrap_or(&600));
            }
        };

        sleep(Duration::from_secs(*backoff_timer.get(backoff_count).unwrap_or(&600))).await;
        backoff_count += 1;
    }
}




#[allow(unused_mut)]
#[tokio::main]
async fn main(){
    dotenv().ok(); 
    simple_logging::log_to_file("log.log", LevelFilter::Info).expect("Couldn't log");
//    const CONF_PATH:String = "~/.config";
    let backoff_timer = vec![10, 20, 30, 60, 120, 300, 600];

    loop {
        let (mut ws_stream, heartbeat_interval) = discord_connection(&backoff_timer).await;
        let (heartbeat_tx, mut rx) = mpsc::channel::<String>(3); 
        let lastfm_tx = heartbeat_tx.clone();
        let (heartbeat_ack_tx, mut heartbeat_ack_rx) = mpsc::channel::<bool>(1); 
        tokio::select!{
            _ = last_fm_poll(lastfm_tx, &backoff_timer) => (),


            failcode = discord_io(heartbeat_ack_tx, rx, ws_stream) => {
                match failcode{
                    9 => {
                        () //Continue
                    },
                    7 => {
                        () //Continue
                    },
                    69 => {
                        () //Continue
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