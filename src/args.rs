use clap::Parser;

#[derive(Debug, Parser)]
// #[clap(author, version, about)]
//
pub struct CommandArgs {
    #[clap(long, env, value_name = "token")]
    /// Your Discord token
    pub discord_token: Option<String>,
    #[clap(long, env, value_name = "username")]
    /// Your last.fm username
    pub lfm_user: Option<String>,
    #[clap(long, env, value_name = "API key")]
    /// Your last.fm API key
    pub lfm_api_key: Option<String>,
    #[clap(long, value_name = "FILE")]
    /// Location of config file
    pub config: Option<String>,
}
