use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

// Persists across commands so we don't relaunch the browser every time.
static BROWSER: Lazy<Mutex<Option<Browser>>> = Lazy::new(|| Mutex::new(None));

/// Get the existing controlled browser, or launch a fresh one.
async fn get_or_launch(exe_path: &str) -> anyhow::Result<()> {
    let mut guard = BROWSER.lock().await;
    if guard.is_some() {
        return Ok(());
    }

    let config = BrowserConfig::builder()
        .chrome_executable(exe_path)
        .with_head() // visible, not headless — user should see it working
        .build()
        .map_err(|e| anyhow::anyhow!(e))?;

    let (browser, mut handler) = Browser::launch(config).await?;

    // Drives the CDP event loop; must stay alive for the browser to function.
    tokio::spawn(async move {
        while let Some(_) = handler.next().await {}
    });

    *guard = Some(browser);
    Ok(())
}

async fn new_page(url: &str) -> anyhow::Result<Page> {
    let guard = BROWSER.lock().await;
    let browser = guard.as_ref().ok_or_else(|| anyhow::anyhow!("browser not launched"))?;
    let page = browser.new_page(url).await?;
    page.wait_for_navigation().await?;
    Ok(page)
}

/// Search on Google and optionally click into the first organic result.
pub async fn search_and_open_first(
    exe_path: &str,
    query: &str,
    open_first: bool,
) -> anyhow::Result<()> {
    get_or_launch(exe_path).await?;

    let url = format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(query)
    );
    let page = new_page(&url).await?;

    if !open_first {
        return Ok(());
    }

    // Google's result title links are h3 inside an <a>; selector may need
    // occasional updates if Google changes markup.
    let link = page
        .find_element("div#search a h3")
        .await
        .map_err(|_| anyhow::anyhow!("no results found"))?;

    // Click the parent <a>, not the h3 itself
    let anchor = link
        .find_element("xpath/..")
        .await
        .map_err(|_| anyhow::anyhow!("could not locate result link"))?;

    anchor.click().await?;
    page.wait_for_navigation().await?;

    Ok(())
}

/// Just open a search results page (reuses persistent browser, no click).
pub async fn search_only(exe_path: &str, query: &str) -> anyhow::Result<()> {
    search_and_open_first(exe_path, query, false).await
}