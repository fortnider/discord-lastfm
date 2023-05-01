use std::time::Duration;
use std::env;
use reqwest;
use dotenv::dotenv;
use tokio_tungstenite::connect_async;
use tungstenite::protocol::Message;
use tungstenite;
use tokio::sync::mpsc;
use futures::{SinkExt,StreamExt};
use serde_json::Value;
use tokio::time::sleep;
use serde_json::json;
use serde_json::Value::Null;




#[tokio::main]
async fn main(){
    let mut restart = true;
    while restart{
        println!("Starting/restarting");
        match discord_lastfm().await {
//            Err => restart = false,
            7 => println!("Opcode 7 received, restarting"),
            _ => {
                restart = false;
                println!("discord_lastfm returned with opcode other than 7, exiting")
            }
        }
    }

}


async fn discord_lastfm() -> i32 {
    #![allow(unused_mut)]
    dotenv().ok(); 
    // Read environment variables

    let (heartbeat_tx, mut rx) = mpsc::channel::<String>(3); 
    // Create a multi-producer, single consumer channel. The producers are the heartbeat and last.fm data, and the reciever is our output loop

    let lastfm_tx = heartbeat_tx.clone();
    let mut status = "online"; // "online", "dnd", "idle"



    // Last FM Portion 
    tokio::spawn(
        async move{
            let mut last_fm_api_key: String = env::var("LAST_FM_API_KEY").unwrap();
            let mut last_fm_user:String = env::var("LAST_FM_USER").unwrap();
            let mut last_fm_url: String = format!("https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user={}&api_key={}&limit=1&format=json", last_fm_user, last_fm_api_key);
            let mut lfm_response:Value;
            let mut poll_time: u64 = 10;
            let mut song: Vec<String> = vec!["".to_string(),"".to_string(),"".to_string()];

            let mut status_update:String;

            

            loop{
                sleep(Duration::from_secs(poll_time)).await;

                lfm_response = reqwest::get(&last_fm_url)
                .await
                .expect("Could not get response from last fm")
                .json()
                .await
                .expect("Could not parse json");
//                println!("{:#?}", lfm_response);


                if lfm_response.pointer("/recenttracks/track/0/name").unwrap().to_string() == song[0] 
                && lfm_response.pointer("/recenttracks/track/0/artist/#text").unwrap().to_string() == song[1]
                && lfm_response.pointer("/recenttracks/track/0/album/#text").unwrap().to_string() == song[2]
                {
//                    println!("Song info hasn't changed, not updating")
                }

                

                else {
                    song= vec![
                    lfm_response.pointer("/recenttracks/track/0/name").unwrap().to_string(),
                    lfm_response.pointer("/recenttracks/track/0/artist/#text").unwrap().to_string(),
                    lfm_response.pointer("/recenttracks/track/0/album/#text").unwrap().to_string(),
                    ];
//                    println!("{} \n {} \n {} \n", song[0], song[1], song[2]);
                    status_update = r#"{"op": 3, "d": {"since": 0,"activities": [{"name": "NAME","type": 2,"id": "custom", "details":"DETAILS"}],"status": "STATUS","afk": false}}"#
                        .replace("STATUS", status)
                        .replace("NAME", format!("{} - {}", &song[1], &song[0]).replace(r#"""#, "").as_str())
                        .replace("DETAILS", &song[2].replace(r#"""#, "").as_str());
//                    println!("{:#?}", status_update);

                
                    lastfm_tx.send(status_update.to_string()).await.expect("couldnt send status update");    //FIX THIS WHEN LEARN DISCORD API
                }
            }
        }
    );

    // Load environment variables from .env file
    let client = reqwest::Client::new();
    let mut token:String = env::var("DISCORD_TOKEN").unwrap();
    const DISCORD_URL: &str = "wss://gateway.discord.gg/?v=9&encoding=json";



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


    let (mut ws_stream, _) = 
        connect_async(DISCORD_URL).await.expect("Failed ws connect");


    let hello: Value = serde_json::from_str(&ws_stream.next().await.unwrap().unwrap().to_string()).unwrap();
    println!("poop {:#?}", hello);
    // Wait until hello from discord is recieved, then turn into json. 


    let heartbeat: u64 = hello.pointer("/d/heartbeat_interval").unwrap().to_string().parse::<u64>().unwrap();
    println!("heartbeat {:?}", heartbeat);
    // Retrieve heartbeat from json. 

    let identity = r#"{"op": 2, "d": {"token": "TOKEN", "properties": {"$os": "Windows 10", "$browser": "Google Chrome", "$device": "Windows"}, "presence": {"status": "STATUS", "afk": false}}, "s": null, "t": null}"#
        .replace("TOKEN", &token)
        .replace("STATUS", &status);

//    println!("{:#?}", identity);

    ws_stream.send(Message::Text(identity.to_string())).await.expect("couldn't send identity... le sigh");
    // Send identity to discord

    // At this point our connection to discord has been established, so what's left is to send heartbeats every $heartbeat ms, retrieve last.fm info, and then send it to discord.

    let (heartbeat_ack_tx, mut heartbeat_ack_rx) = mpsc::channel::<bool>(1);

    tokio::spawn(
        // Spawns a thread that sends the heartbeat json to our mpsc channel every $heartbeat ms
        // If heartbeat is not acked from discord, returns loop. 
        async move {
            sleep(Duration::from_millis(heartbeat)).await;
            loop{
                heartbeat_tx.send(r#"{"op": 1, "d": "None"}"#.to_string()).await.expect("Failed to send heartbeat to mspc channel");
//                println!("heartbeat send... waiting");
                sleep(Duration::from_millis(heartbeat)).await;
                if heartbeat_ack_rx.try_recv().unwrap_or(false) == false {
                    println!("No heartbeat detected! Returning");
                    return 99
                };
            }
    
        } 
    );







    loop{ // Main websocket send/receive loop
        tokio::select!{
            ws_in = ws_stream.next() => {
                let msg_str = ws_in.unwrap().unwrap().to_string();
                let msg_json: Value = serde_json::from_str(&msg_str).unwrap_or(json!(Null));
                // Turn discord reponse to json. If fail, return Null
//                println!("RECIEVED => {:#?}", msg_json);
                let opcode = &msg_json["op"].to_string().parse::<i32>().unwrap_or(99);
                // Extract the opcode from the json response. If there isn't a opcode, set opcode to 99
                match opcode{
                    11 => { //11 is discord's heartbeat ack
                        heartbeat_ack_tx.send(true).await.expect("Could not send message through heartbeat_ack");
//                        println!("Recieved heartbeat ack, sent mpsc message")
                    }, 
                    0 => { // 0 is discord receiving whatever you sent
//                        println!("discord received")
                    },
                    _ => () //println!("Not heartbeat"),


                }

                
            },

            ws_out = rx.recv() => {
                match ws_out{
                    Some(ws_out) => {
                    //    println!("sending: \n {:#?}", ws_out);
                        let _ = ws_stream.send(Message::Text(ws_out)).await;
                    },

                    None => break
                }


            }

        }
    } 
    return 99
}
