use std::time::Duration;
use std::env;
use log::info;
use reqwest;
use dotenv::dotenv;
use tokio::net::TcpStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tungstenite::protocol::Message;
use tungstenite;
use tokio::sync::{mpsc};
use futures::{SinkExt,StreamExt};
use serde_json::Value;
use tokio::time::sleep;
use serde_json::json;
use serde_json::Value::Null;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::Receiver;
use log::LevelFilter;
use log::debug;
use log::warn;
use log::error;



#[allow(unused_mut)]
async fn last_fm_poll(lastfm_tx: Sender<String>){

    dotenv().ok(); 
    // Read environment variables

    // Create a multi-producer, single consumer channel. The producers are the heartbeat and last.fm data, and the reciever is our output loop

    let mut last_fm_api_key: String = env::var("LAST_FM_API_KEY").unwrap();
    let mut last_fm_user:String = env::var("LAST_FM_USER").unwrap();
    let mut last_fm_url: String = format!("https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user={}&api_key={}&limit=1&format=json", last_fm_user, last_fm_api_key);
    let mut poll_time: u64 = 10;
    let mut status = "online"; // "online", "dnd", "idle"
    // In the future will be editable using GUI, so keeping it mut. 

    let mut lfm_response:Value;
    let mut song: Vec<String> = vec!["".to_string(),"".to_string(),"".to_string()];
    let mut status_update:String;
    let mut status_cleared = true;
    // Variables for later


    loop{
        sleep(Duration::from_secs(poll_time)).await;
        debug!("Polling Last FM");
        lfm_response = reqwest::get(&last_fm_url)
        .await
        .expect("Could not get response from last fm")
        .json()
        .await
        .expect("Could not parse json");
        //debug!("LastFM Returned {:#?}", lfm_response);
        if lfm_response["recenttracks"]["track"][0]["@attr"]["nowplaying"].to_string() == r#""true""#
        // If the song is now playing
        {

            if lfm_response.pointer("/recenttracks/track/0/name").unwrap().to_string() != song[0] 
            || lfm_response.pointer("/recenttracks/track/0/artist/#text").unwrap().to_string() != song[1]
            || lfm_response.pointer("/recenttracks/track/0/album/#text").unwrap().to_string() != song[2]{
                // If song is different than previous song

                info!("Song is currently playing and different. Updating status.");
                song= vec![
                lfm_response.pointer("/recenttracks/track/0/name").unwrap().to_string(),
                lfm_response.pointer("/recenttracks/track/0/artist/#text").unwrap().to_string(),
                lfm_response.pointer("/recenttracks/track/0/album/#text").unwrap().to_string(),
                ];
    
    
                status_update = r#"{"op": 3, "d": {"since": 0,"activities": [{"name": "NAME","type": 2,"id": "custom", "details":"DETAILS"}],"status": "STATUS","afk": false}}"#
                // ,"status": "STATUS","afk": false
                .replace("STATUS", status)
                .replace("NAME", format!("{} - {}", &song[0], &song[1]).replace(r#"""#, "").as_str())
                .replace("DETAILS", format!("From {}", &song[2]).replace(r#"""#, "").as_str());
                lastfm_tx.send(status_update.to_string()).await.expect("couldnt send status update"); 
                status_cleared = false;
            
            
            }
            
            
        }
        else if status_cleared == false
        // If song is not now playing
            {
                debug!("Song is not currently playing and status has not been cleared. Clearing status now. ");
                status_update = r#"{"op": 3, "d": {"since": 0,"activities": null,"status": "STATUS","afk": false}}"#.to_string();
                lastfm_tx.send(status_update.to_string()).await.expect("couldnt send status update"); 
                status_cleared = true;

            }
        else {
            //debug!("Song is not currently playing.");
        }
    }
}


async fn discord_io(heartbeat_ack_tx: Sender<bool>, mut rx: Receiver<String>, mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>) -> u64{


    loop{ // Main websocket send/receive loop
        tokio::select!{
            ws_in = ws_stream.next() => {
                match ws_in{
                    Some(_) => {
                        //let msg_str = ws_in.unwrap().unwrap_or("fds".into()).to_string();
                        let msg_json: Value = serde_json::from_str(ws_in.unwrap().unwrap().to_text().unwrap()).unwrap_or(json!(Null));
                        // Turn discord reponse to json. If fail, return Null

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
            warn!("No heartbeat detected! Returning");
            return

        };
    }
}

#[allow(unused_mut)]
#[tokio::main]
async fn main(){
    dotenv().ok(); 
    simple_logging::log_to_file("log.log", LevelFilter::Info).expect("Couldn't log");
    loop {
        let client = reqwest::Client::new();
        let mut token:String = env::var("DISCORD_TOKEN").unwrap();

        let mut status = "online"; // "online", "dnd", "idle"


        let res = client
            .get("https://discordapp.com/api/v9/users/@me")
            .header("Authorization", &token)
            .send()
            .await
            .expect("Could not connect to discord!");

        if res.status().to_string() == "200 OK" {
            info!("Token Valid");
        } else {
            warn!("Token Invalid!! \n{}", res.status())
        };
        //Connect to discord using token. If it fails, print error. Otherwise, token is valid. 

        let (mut ws_stream, _) = 
            connect_async("wss://gateway.discord.gg/?v=9&encoding=json").await.expect("Failed ws connect");

        let hello: Value = serde_json::from_str(&ws_stream.next().await.unwrap().unwrap().to_string()).unwrap();
        // Wait until hello from discord is recieved, then turn into json. 
        info!("Received hello: {:#?}", hello );

        let heartbeat_interval: u64 = hello.pointer("/d/heartbeat_interval").unwrap().to_string().parse::<u64>().unwrap();
        info!("Heartbeat: {:?}", heartbeat_interval);
        // Retrieve heartbeat from json. 

        let identity = r#"{"op": 2, "d": {"token": "TOKEN", "properties": {"$os": "Windows 10", "$browser": "Google Chrome", "$device": "Windows"}, "presence": {"status": "STATUS", "afk": false}}, "s": null, "t": null}"#
            .replace("TOKEN", &token)
            .replace("STATUS", &status);

        ws_stream.send(Message::Text(identity.to_string())).await.expect("couldn't send identity... le sigh");
        info!("Sent Identity: {:#?}", identity);


        let (heartbeat_tx, mut rx) = mpsc::channel::<String>(3); 
        let lastfm_tx = heartbeat_tx.clone();
        let (heartbeat_ack_tx, mut heartbeat_ack_rx) = mpsc::channel::<bool>(1);
            
        tokio::select!{
            _ = last_fm_poll(lastfm_tx) => (),
            failcode = discord_io(heartbeat_ack_tx, rx, ws_stream) => {
                match failcode{
                    9 => {
                        ()
                    },
                    7 => {
                        ()
                    },
                    _ => {
                        error!("Program returned failcode {}", failcode);
                        break
                    },
                }
            }, 
            _ = heartbeat(heartbeat_tx, heartbeat_interval, heartbeat_ack_rx) => (),
            // These three functions should never finish before the cancellation mpsc. When cancellation mpsc receives message, everything is dropped and theoretically should restart. 
        }

        error!("Something happened, Program terminating");
    }
}