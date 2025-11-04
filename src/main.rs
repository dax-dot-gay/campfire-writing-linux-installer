use std::collections::HashMap;

use anyhow::anyhow;
use chrono::Utc;
use hex::ToHex;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

const RELEASES_URL: &'static str = "https://storage.googleapis.com/blaze-desktop-releases";
const LATEST_FILE: &'static str = "latest.yml";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Contents {
    pub key: String,
    pub generation: u64,
    pub meta_generation: u64,
    pub last_modified: chrono::DateTime<Utc>,

    #[serde(rename = "ETag")]
    pub hash: String,
    pub size: u64
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ListBucket {
    pub name: String,
    pub prefix: String,
    pub marker: String,
    pub is_truncated: bool,
    pub contents: Vec<Contents>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LatestFile {
    pub url: String,
    #[serde(rename = "sha512")]
    pub hash: String,
    pub size: u64
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Latest {
    pub version: String,
    pub path: String,
    #[serde(default)]
    pub files: Vec<LatestFile>,

    #[serde(rename = "sha512")]
    pub hash: String,
    pub release_date: chrono::DateTime<Utc>
}

pub async fn get_manifest() -> anyhow::Result<HashMap<String, Contents>> {
    let response = reqwest::get(RELEASES_URL).await?.error_for_status()?;
    let response_text = response.text().await?;
    let bucket = quick_xml::de::from_str::<ListBucket>(&response_text)?;
    let contents: HashMap<String, Contents> = bucket.contents.clone().into_iter().map(|v| (v.key.clone(), v)).collect();
    Ok(contents)
}

pub async fn parse_latest(latest: Contents) -> anyhow::Result<Latest> {
    let response = reqwest::get(format!("{}/{}", RELEASES_URL, LATEST_FILE)).await?.error_for_status()?;
    let latest_bytes = response.bytes().await?;
    let digest = Md5::digest(latest_bytes.clone()).to_vec();
    let to_verify = hex::decode(latest.hash.trim_matches('"'))?;
    
    if to_verify != digest {
        return Err(anyhow!("Downloaded latest.yml failed to verify! Aborting..."));
    }

    let latest = serde_norway::from_slice::<Latest>(&latest_bytes)?;

    Ok(latest)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Downloading release manifest...");
    let contents = get_manifest().await?;

    println!("Retrieved manifest, parsing and verifying latest version...");
    let latest = parse_latest(contents.get(LATEST_FILE).ok_or(anyhow!("File <latest.yml> not found in manifest."))?.clone()).await?;

    println!("{latest:?}");


    Ok(())
}
