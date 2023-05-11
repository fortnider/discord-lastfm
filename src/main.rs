use std::time::Duration;
use std::env;
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
        println!("Polling last.fm");
        lfm_response = reqwest::get(&last_fm_url)
        .await
        .expect("Could not get response from last fm")
        .json()
        .await
        .expect("Could not parse json");
//                println!("{:#?}", lfm_response);
        if lfm_response["recenttracks"]["track"][0]["@attr"]["nowplaying"].to_string() == r#""true""#
        // If the song is now playing
        {

            if lfm_response.pointer("/recenttracks/track/0/name").unwrap().to_string() != song[0] 
            || lfm_response.pointer("/recenttracks/track/0/artist/#text").unwrap().to_string() != song[1]
            || lfm_response.pointer("/recenttracks/track/0/album/#text").unwrap().to_string() != song[2]{
                // If song is different than previous song

                println!("Song is currently playing and different. Updating status. ");
                println!("{}", lfm_response["recenttracks"]["track"][0]["@attr"]["nowplaying"].to_string());
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
                println!("Song is not currently playing and status has not been cleared. Clearing status now. ");
                status_update = r#"{"op": 3, "d": {"since": 0,"activities": null,"status": "STATUS","afk": false}}"#.to_string();
                lastfm_tx.send(status_update.to_string()).await.expect("couldnt send status update"); 
                status_cleared = true;

            }
        else {
            println!("Song is not currently playing.");
        }
    }
}


async fn discord_io(heartbeat_ack_tx: Sender<bool>, mut rx: Receiver<String>, mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>) {


    loop{ // Main websocket send/receive loop
        tokio::select!{
            ws_in = ws_stream.next() => {
                match ws_in{
                    Some(_) => {
                        let msg_str = ws_in.unwrap().unwrap_or("fds".into()).to_string();
                        let msg_json: Value = serde_json::from_str(&msg_str).unwrap_or(json!(Null));

                        // Turn discord reponse to json. If fail, return Null

                        let opcode = &msg_json["op"].to_string().parse::<i32>().unwrap_or(99);
                        // Extract the opcode from the json response. If there isn't a opcode, set opcode to 99
                        match opcode{
                            11 => { //11 is discord's heartbeat ack
                                heartbeat_ack_tx.send(true).await.expect("Could not send message through heartbeat_ack");
                              println!("Recieved heartbeat ack, sent mpsc message")
                            }, 

                            0 => { // 0 is discord receiving whatever you sent
//                              println!("discord received")
                            },

                            _ => println!("{}", msg_str) ,
                        }

                    },
                    None => (),

                }
                
            },

            ws_out = rx.recv() => {
                match ws_out{
                    Some(ws_out) => {
                        println!("Sending: \n {:#?}", ws_out);
                        let _ = ws_stream.send(Message::Text(ws_out)).await;
                    },

                    None => break
                }


            },
        }

    }
}

async fn heartbeat(heartbeat_tx: Sender<String>, heartbeat_interval: u64, mut heartbeat_ack_rx: Receiver<bool>){
    sleep(Duration::from_millis(heartbeat_interval)).await;
    loop{
        heartbeat_tx.send(r#"{"op": 1, "d": "None"}"#.to_string()).await.expect("Failed to send heartbeat to mspc channel");
        println!("Heartbeat Sent");
        sleep(Duration::from_millis(heartbeat_interval)).await;
        if heartbeat_ack_rx.try_recv().unwrap_or(false) == false {
            println!("No heartbeat detected! Returning");
        };
    }
}

#[allow(unused_mut)]
#[tokio::main]
async fn main(){
    dotenv().ok(); 

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
        println!("Token Valid");
    } else {
        println!("Token Invalid!! \n{}", res.status())
    };

    //Connect to discord using token. If it fails, print error. Otherwise, token is valid. 


//    const DISCORD_URL: &str = "wss://gateway.discord.gg/?v=9&encoding=json";
    let (mut ws_stream, _) = 
        connect_async("wss://gateway.discord.gg/?v=9&encoding=json").await.expect("Failed ws connect");

    let hello: Value = serde_json::from_str(&ws_stream.next().await.unwrap().unwrap().to_string()).unwrap();
//    println!("{:#?}", hello);
    // Wait until hello from discord is recieved, then turn into json. 


    let heartbeat_interval: u64 = hello.pointer("/d/heartbeat_interval").unwrap().to_string().parse::<u64>().unwrap();
    println!("heartbeat {:?}", heartbeat_interval);
    // Retrieve heartbeat from json. 

    let identity = r#"{"op": 2, "d": {"token": "TOKEN", "properties": {"$os": "Windows 10", "$browser": "Google Chrome", "$device": "Windows"}, "presence": {"status": "STATUS", "afk": false}}, "s": null, "t": null}"#
        .replace("TOKEN", &token)
        .replace("STATUS", &status);

//    println!("{:#?}", identity);

    ws_stream.send(Message::Text(identity.to_string())).await.expect("couldn't send identity... le sigh");



    let (heartbeat_tx, mut rx) = mpsc::channel::<String>(3); 
    let lastfm_tx = heartbeat_tx.clone();
    let (heartbeat_ack_tx, mut heartbeat_ack_rx) = mpsc::channel::<bool>(1);
    tokio::select!{
        _ = last_fm_poll(lastfm_tx) => (),
        _ = discord_io(heartbeat_ack_tx, rx, ws_stream) => (), 
        _ = heartbeat(heartbeat_tx, heartbeat_interval, heartbeat_ack_rx) => (),
        // These three functions should never finish before the cancellation mpsc. When cancellation mpsc receives message, everything is dropped and theoretically should restart. 
    }
    println!("end");

}