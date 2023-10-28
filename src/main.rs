use std::sync::Arc;
use std::env;

use once_cell::sync::Lazy;
use reqwest::Error;
use serde::{Deserialize, Serialize};
use serenity::builder::CreateApplicationCommand;
use serenity::model::gateway::Ready;
use serenity::model::prelude::command::Command;
use serenity::model::prelude::GuildId;
use serenity::model::Permissions;
use serenity::prelude::*;
use serenity::utils::Color;
use serenity::{async_trait, model::prelude::Interaction};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};

struct Bot;

static TOKEN: Lazy<Arc<Mutex<String>>> = Lazy::new(|| Arc::new(Mutex::new(String::new())));
static GUILDS: Lazy<Arc<Mutex<Vec<GuildId>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IncidentAction {
    invites_disabled_until: Option<String>,
    dms_disabled_until: Option<String>,
}

#[async_trait]
impl EventHandler for Bot {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        log::info!("Interaction: {:?}", interaction);
        let interaction = interaction.as_application_command();
        if interaction.is_none() {
            return;
        }
        let interaction = interaction.unwrap();
        if interaction.data.name.eq("instant") {
            interaction.defer(&ctx.http).await.ok();
            let guild_id = interaction.guild_id.unwrap();
            let res = enable_security_actions(guild_id).await;
            if res.is_err() || !res.as_ref().unwrap() {
                log::error!("Failed to enable security actions for guild {}", guild_id.0);
                log::error!("{:?}", res.unwrap_err());
                interaction.edit_original_interaction_response(&ctx, |f| {
                    f.embed(|e| e.title("Error").description("Failed to enable security actions. Maybe permissions are missing?").color(Color::RED))
                }).await.ok();
            } else {
                interaction
                    .edit_original_interaction_response(&ctx, |f| {
                        f.embed(|e| {
                            e.title("Success")
                                .description("Enabled security actions for 24 hours.")
                                .color(Color::DARK_GREEN)
                        })
                    })
                    .await
                    .ok();
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        log::info!("{} is connected!", ready.user.name);
        Command::set_global_application_commands(&ctx.http, |command| {
            command.set_application_commands(vec![CreateApplicationCommand::default()
                .name("instant")
                .description("Instant Enable security actions")
                .default_member_permissions(Permissions::MANAGE_GUILD)
                .to_owned()
                .dm_permission(false)
                .to_owned()])
        })
        .await
        .ok();
    }

    async fn guild_create(&self, _ctx: Context, guild: serenity::model::guild::Guild, _is_new: bool) {
        log::info!("Guild: {:?}", guild);
        GUILDS.lock().await.push(guild.id);
    }
}

async fn enable_security_actions(guild_id: GuildId) -> Result<bool, Error> {
    let client = reqwest::Client::new();
    let token = TOKEN.lock().await.clone();
    let url = format!(
        "https://discord.com/api/v9/guilds/{}/incident-actions",
        guild_id.0
    );
    let body = IncidentAction {
        invites_disabled_until: None,
        dms_disabled_until: Some((chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339()),
    };

    let res = client
        .post(&url)
        .header("Authorization", format!("Bot {}", token))
        .json(&body)
        .send()
        .await?;

    let json = res.json::<IncidentAction>().await?;

    if json.dms_disabled_until.is_some() {
        log::info!("Enabled security actions for guild {}", guild_id.0);
    } else {
        log::error!("Failed to enable security actions for guild {}", guild_id.0);
        return Ok(false);
    }

    Ok(true)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    env_logger::builder()
        .filter_module("no-dm-forever", {
            if cfg!(debug_assertions) {
                log::LevelFilter::Trace
            } else {
                log::LevelFilter::Info
            }
        })
        .init();
    let sched = JobScheduler::new().await.unwrap();

    let token = if let Some(token) = env::var("DISCORD_TOKEN").ok() {
        token
    } else {
        panic!("'DISCORD_TOKEN' was not found");
    };
    TOKEN.lock().await.push_str(&token);

    let intents = GatewayIntents::GUILDS;

    let mut client = Client::builder(&token, intents)
        .event_handler(Bot)
        .await
        .expect("Err creating client");
    
    sched.add(
        Job::new_async("1/7 * * * * *", |_uuid, mut _l| {
            Box::pin(async move {
                for guild in GUILDS.lock().await.iter() {
                    let res = enable_security_actions(*guild).await;
                    if res.is_err() || !res.as_ref().unwrap() {
                        log::error!("Failed to enable security actions for guild {}", guild.0);
                        if res.is_err() {
                            log::error!("{:?}", res.unwrap_err());
                        }
                    }
                    log::info!("Enabled security actions for guild {}", guild.0);

                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                }
            })
        })
        .unwrap()
    ).await.unwrap();


    sched.start().await.unwrap();

    client.start_autosharded().await.unwrap();
}
