const RELEASES_URL: &'static str = "https://storage.googleapis.com/blaze-desktop-releases";
const LATEST_FILE: &'static str = "latest.yml";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let response = reqwest::get(RELEASES_URL).await?.error_for_status()?;
    println!("{:?}", response.text().await);

    Ok(())
}
