//! `agent-of-empires profile` subcommands implementation

use anyhow::Result;
use clap::Subcommand;
use std::io::{self, Write};

use crate::session;

#[derive(Subcommand)]
pub enum ProfileCommands {
    /// List all profiles
    #[command(alias = "ls")]
    List,

    /// Create a new profile
    #[command(alias = "new")]
    Create {
        /// Profile name
        name: String,
    },

    /// Delete a profile
    #[command(alias = "rm")]
    Delete {
        /// Profile name
        name: String,
    },

    /// Rename a profile
    #[command(alias = "mv")]
    Rename {
        /// Current profile name
        old_name: String,
        /// New profile name
        new_name: String,
    },
}

pub async fn run(command: Option<ProfileCommands>) -> Result<()> {
    match command {
        Some(ProfileCommands::List) | None => list_profiles().await,
        Some(ProfileCommands::Create { name }) => create_profile(&name).await,
        Some(ProfileCommands::Delete { name }) => delete_profile(&name).await,
        Some(ProfileCommands::Rename { old_name, new_name }) => {
            rename_profile(&old_name, &new_name).await
        }
    }
}

async fn list_profiles() -> Result<()> {
    let profiles = session::list_profiles()?;

    if profiles.is_empty() {
        println!("No profiles found.");
        return Ok(());
    }

    println!("Profiles:");
    for p in &profiles {
        println!("    {}", p);
    }
    println!("\nTotal: {} profiles", profiles.len());

    Ok(())
}

async fn create_profile(name: &str) -> Result<()> {
    session::create_profile(name)?;
    println!("✓ Created profile: {}", name);
    println!("  Use with: agent-of-empires -p {}", name);
    Ok(())
}

async fn rename_profile(old_name: &str, new_name: &str) -> Result<()> {
    session::rename_profile(old_name, new_name)?;
    println!("✓ Renamed profile: {} -> {}", old_name, new_name);
    Ok(())
}

async fn delete_profile(name: &str) -> Result<()> {
    print!(
        "Are you sure you want to delete profile '{}'? This will remove all sessions in this profile. [y/N] ",
        name
    );
    io::stdout().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;

    if response.trim().to_lowercase() != "y" {
        println!("Cancelled.");
        return Ok(());
    }

    session::delete_profile(name)?;
    println!("✓ Deleted profile: {}", name);
    Ok(())
}
