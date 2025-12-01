use std::{collections::HashMap, path::PathBuf};

use anyhow::anyhow;
use chrono::Utc;
use clap::{Parser, ValueEnum};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::process::Command;
use tempdir::TempDir;
use tokio::fs;

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
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ListBucket {
    pub name: String,
    pub prefix: String,
    pub marker: String,
    pub is_truncated: bool,
    pub contents: Vec<Contents>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LatestFile {
    pub url: String,
    #[serde(rename = "sha512")]
    pub hash: String,
    pub size: u64,
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
    pub release_date: chrono::DateTime<Utc>,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AppArch {
    X86,
    Arm,
}

impl AppArch {
    pub fn fname(&self) -> String {
        match self {
            AppArch::X86 => "app-64.7z",
            AppArch::Arm => "app-arm64.7z",
        }
        .to_string()
    }

    pub fn dname(&self) -> String {
        match self {
            AppArch::X86 => "app-64",
            AppArch::Arm => "app-arm64",
        }
        .to_string()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Parser)]
#[command(version, about)]
pub struct Args {
    /// Point at an setup.exe
    #[arg(long)]
    pub existing: Option<String>,

    /// Application architecture to extract
    #[arg(long, short, value_enum, default_value_t=AppArch::X86)]
    pub architecture: AppArch,
}

pub async fn get_manifest() -> anyhow::Result<HashMap<String, Contents>> {
    let response = reqwest::get(RELEASES_URL).await?.error_for_status()?;
    let response_text = response.text().await?;
    let bucket = quick_xml::de::from_str::<ListBucket>(&response_text)?;
    let contents: HashMap<String, Contents> = bucket
        .contents
        .clone()
        .into_iter()
        .map(|v| (v.key.clone(), v))
        .collect();
    Ok(contents)
}

pub async fn parse_latest(latest: Contents) -> anyhow::Result<Latest> {
    let response = reqwest::get(format!("{}/{}", RELEASES_URL, LATEST_FILE))
        .await?
        .error_for_status()?;
    let latest_bytes = response.bytes().await?;
    let digest = Md5::digest(latest_bytes.clone()).to_vec();
    let to_verify = hex::decode(latest.hash.trim_matches('"'))?;

    if to_verify != digest {
        return Err(anyhow!(
            "Downloaded latest.yml failed to verify! Aborting..."
        ));
    }

    let latest = serde_norway::from_slice::<Latest>(&latest_bytes)?;

    Ok(latest)
}

pub async fn download_verify_latest(
    latest: Latest,
    contents: HashMap<String, Contents>,
) -> anyhow::Result<TempDir> {
    let latest_file = latest
        .files
        .get(0)
        .expect("Expected at least 1 listed file for the latest version!")
        .clone();
    let fetch_response = reqwest::get(format!("{}/{}", RELEASES_URL, latest_file.url))
        .await?
        .error_for_status()?;
    let downloaded_bytes = fetch_response.bytes().await?;

    println!("Downloaded, waiting to verify...");

    let digest = Md5::digest(downloaded_bytes.clone()).to_vec();
    let to_verify = hex::decode(
        contents
            .get(&latest_file.url)
            .expect("Should be a match in content list!")
            .clone()
            .hash
            .trim_matches('"'),
    )?;
    if to_verify != digest {
        return Err(anyhow!(
            "Downloaded setup exe failed to verify! Aborting..."
        ));
    }

    let workdir = TempDir::new_in(".", "camp-li-")?;
    fs::write(workdir.path().join("setup.exe"), downloaded_bytes).await?;

    Ok(workdir)
}

pub fn extract_files(directory: &TempDir, args: Args) -> anyhow::Result<String> {
    let cmd = Command::new("unar")
        .args([
            "-o",
            directory.path().to_str().unwrap(),
            "-d",
            directory.path().join("setup.exe").to_str().unwrap(),
        ])
        .status()?;
    if cmd.success() {
        let mut results: Vec<String> = rust_search::SearchBuilder::default()
            .location(directory.path().join("setup"))
            .search_input(args.architecture.fname())
            .limit(1)
            .strict()
            .depth(3)
            .build()
            .collect();
        if let Some(found) = results.pop() {
            let status = Command::new("unar")
                .args([
                    "-o",
                    directory.path().to_str().unwrap(),
                    "-d",
                    found.as_str(),
                ])
                .status()?;
            if status.success() {
                let mut results: Vec<String> = rust_search::SearchBuilder::default()
                    .location(directory.path().join(args.architecture.dname()))
                    .search_input("app.asar")
                    .limit(1)
                    .strict()
                    .depth(3)
                    .build()
                    .collect();

                if let Some(asar) = results.pop() {
                    Ok(asar)
                } else {
                    Err(anyhow!("Failed to find ASAR!"))
                }
            } else {
                Err(anyhow!("Internal extraction failed with code: {status}"))
            }
        } else {
            Err(anyhow!(
                "Failed to find internal archive: {}",
                args.architecture.fname()
            ))
        }
    } else {
        Err(anyhow!("Initial extraction failed with code: {cmd}"))
    }
}

pub fn extract_asar(directory: &TempDir, asar: String) -> anyhow::Result<PathBuf> {
    let status = Command::new("npx")
        .args([
            "--yes",
            "@electron/asar",
            "extract",
            asar.as_str(),
            directory.path().join("unpack").to_str().unwrap(),
        ])
        .status()?;
    if status.success() {
        Ok(directory.path().join("unpack").to_path_buf())
    } else {
        Err(anyhow!("Failed to extract asar with code: {status}"))
    }
}

pub async fn patch_file(directory: &TempDir) -> anyhow::Result<()> {
    let fcontent = fs::read_to_string(directory.path().join("unpack/main.js")).await?;
    let patched = fcontent
        .replace(
            "setupAutoUpdate(tabs);",
            "// PATCH: REMOVE <setupAutoUpdate(tabs);>",
        )
        .replace(
            "let confirmedLatestVersion = isDev;",
            "let confirmedLatestVersion = true;",
        );
    fs::write(
        directory.path().join("unpack/main.js"),
        patched.into_bytes(),
    )
    .await?;

    let pcontent = fs::read_to_string(directory.path().join("unpack/package.json")).await?;
    let ppatched = pcontent.replace("\"@campfire-technology-llc/writing-software\": \"*\",", "");
    fs::write(
        directory.path().join("unpack/package.json"),
        ppatched.into_bytes(),
    )
    .await?;
    Ok(())
}

pub async fn build_and_distribute(directory: &TempDir) -> anyhow::Result<()> {
    Command::new("npm")
        .current_dir(directory.path().join("unpack"))
        .args(["install", "--save-dev", "electron"])
        .status()?;
    Command::new("npm")
        .current_dir(directory.path().join("unpack"))
        .args(["install", "--save", "uuid"])
        .status()?;
    Command::new("npx")
        .current_dir(directory.path().join("unpack"))
        .args(["electron-builder", "-l", "tar.xz", "appimage"])
        .status()?;
    copy_dir::copy_dir(directory.path().join("unpack/dist"), "output")?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let workdir = if let Some(existing) = args.existing.clone() {
        println!("Skipping download, file exists...");
        let workdir = TempDir::new_in(".", "camp-li-")?;
        std::fs::copy(existing, workdir.path().join("setup.exe"))?;
        workdir
    } else {
        println!("Downloading release manifest...");
        let contents = get_manifest().await?;

        println!("Retrieved manifest, parsing and verifying latest version...");
        let latest = parse_latest(
            contents
                .get(LATEST_FILE)
                .ok_or(anyhow!("File <latest.yml> not found in manifest."))?
                .clone(),
        )
        .await?;

        let workdir = download_verify_latest(latest, contents.clone()).await?;

        println!("Downloaded setup executable!");
        workdir
    };

    println!("Working in: {workdir:?}");
    let found_path = extract_files(&workdir, args.clone())?;
    println!("Internal archive: {found_path}");

    let unpack_path = extract_asar(&workdir, found_path)?;
    println!("Unpacked to: {:?}. Patching...", unpack_path.clone());

    patch_file(&workdir).await?;
    build_and_distribute(&workdir).await?;

    Ok(())
}
