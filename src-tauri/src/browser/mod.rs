// src-tauri/src/browser/mod.rs
// Cargo: chromiumoxide = { version = "0.7", features = ["tokio-runtime"], default-features = false }
//        futures = "0.3", serde = { version = "1", features = ["derive"] }, serde_json, anyhow, tokio
//
// Planned layout:
//   browser/mod.rs      <- this file: driver + snapshot + actions
//   browser/skills/     <- amazon.rs, flipkart.rs (URL builders + extract)
//   browser/agent.rs    <- generic observe->LLM->act loop (next step)

use crate::system::helper_functions::{resolve_app, silent_command};
use anyhow::{anyhow, Result};
use chromiumoxide::{browser::BrowserConfig, Browser as Cdp, Page};
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};
use futures::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Serialize, Deserialize)]
pub struct El {
    pub id: u32,
    pub role: String,
    pub label: String,
    pub value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub url: String,
    pub title: String,
    pub text: String,
    pub elements: Vec<El>,
}

impl Snapshot {
    /// Compact text form for the LLM.
    pub fn render(&self) -> String {
        let mut s = format!(
            "URL: {}\nTITLE: {}\nTEXT:\n{}\nELEMENTS:\n",
            self.url, self.title, self.text
        );
        for e in &self.elements {
            s += &format!(
                "[{}] {} \"{}\"{}\n",
                e.id,
                e.role,
                e.label,
                e.value
                    .as_ref()
                    .map(|v| format!(" = {v}"))
                    .unwrap_or_default()
            );
        }
        s
    }
}

// Tags visible interactive elements with data-aid so we can target them by id.
const SNAPSHOT_JS: &str = r#"(() => {
  const sel = 'a[href],button,input,select,textarea,[role=button],[role=checkbox],[role=link],[role=option],[role=tab]';
  const vis = e => { const r = e.getBoundingClientRect(), s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  const els = []; let n = 0;
  document.querySelectorAll('[data-aid]').forEach(e => e.removeAttribute('data-aid'));
  for (const e of document.querySelectorAll(sel)) {
    if (!vis(e) || n >= 80) continue;
    const label = (e.getAttribute('aria-label') || e.innerText || e.placeholder || e.title || e.value || '')
      .replace(/\s+/g, ' ').trim().slice(0, 80);
    if (!label && e.tagName !== 'INPUT') continue;
    e.setAttribute('data-aid', ++n);
    const role = e.getAttribute('role') || (e.tagName === 'A' ? 'link' : e.tagName === 'INPUT' ? (e.type || 'input') : e.tagName.toLowerCase());
    els.push({ id: n, role, label, value: e.tagName === 'A' ? (e.href || '').slice(0, 120) : (e.value || null) });
  }
  const text = (document.body.innerText || '').replace(/\n{2,}/g, '\n').slice(0, 3000);
  return { url: location.href, title: document.title, text, elements: els };
})()"#;

pub struct BrowserSession {
    _cdp: Cdp,
    page: Mutex<Page>, // tokio::sync::Mutex, already imported
}

impl BrowserSession {
    /// Launch visible Chrome with a persistent profile (keeps logins/cookies).
    // 1. launch() takes the exe path
    pub async fn launch(exe_path: &str, profile_dir: &str) -> Result<Self> {
        let cfg = BrowserConfig::builder()
            .chrome_executable(exe_path)
            .with_head()
            .viewport(None) // use the real window size
            .arg("start-maximized")
            .args([
                "no-first-run",
                "no-default-browser-check",
                "disable-session-crashed-bubble",
            ])
            .user_data_dir(profile_dir)
            .build()
            .map_err(|e| anyhow!(e))?;

        let (cdp, mut handler) = Cdp::launch(cfg).await?;
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        let mut page = None;
        for _ in 0..10 {
            if let Some(p) = cdp.pages().await?.into_iter().next() {
                page = Some(p);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        let page = match page {
            Some(p) => p,
            None => cdp.new_page("about:blank").await?,
        };
        Ok(Self {
            _cdp: cdp,
            page: Mutex::new(page),
        })
    }

    async fn pg(&self) -> Page {
        self.page.lock().await.clone()
    }

    pub async fn tab(&self, cmd: &str) -> Result<String> {
        let pages = self._cdp.pages().await?;
        let n = pages.len();
        // current = the tab the user actually sees (they may have clicked around by hand)
        let mut cur = 0;
        for (i, p) in pages.iter().enumerate() {
            let v = p
                .evaluate("document.visibilityState")
                .await
                .ok()
                .and_then(|r| r.into_value::<String>().ok());
            if v.as_deref() == Some("visible") {
                cur = i;
                break;
            }
        }
        let idx = match cmd.trim() {
            "next" => (cur + 1) % n,
            "prev" => (cur + n - 1) % n,
            "first" => 0,
            "last" => n - 1,
            d => d
                .parse::<usize>()
                .ok()
                .and_then(|k| k.checked_sub(1))
                .filter(|&i| i < n)
                .ok_or_else(|| anyhow!("only {n} tabs open"))?,
        };
        let p = pages[idx].clone();
        p.bring_to_front().await?;
        *self.page.lock().await = p.clone(); // later actions now act on this tab
        Ok(p.get_title().await?.unwrap_or_default())
    }

    /// Organic Google results in on-screen order: (title, url)
    pub async fn results(&self) -> Result<Vec<(String, String)>> {
        const JS: &str = r#"(() => {
            const seen = new Set(), out = [];
            for (const a of document.querySelectorAll('a[href]')) {
                const t = (a.innerText || '').trim();
                if (!/https?:\/\//.test(t)) continue;          // main results show their URL
                let u;
                try { u = new URL(a.href); } catch (e) { continue; }
                const isGoogle = /^(www\.)?google\./.test(u.hostname);
                if (isGoogle && !u.pathname.startsWith('/goto')) continue;
                if (seen.has(a.href)) continue;
                seen.add(a.href);
                const title = t.split('\n').map(s => s.trim()).filter(Boolean)[0];
                out.push([title, a.href]);
            }
            return out;
        })()"#;

        for _ in 0..12 {
            let v = self.eval_json(JS).await?;
            let r: Vec<(String, String)> = serde_json::from_value(v)?;
            if !r.is_empty() {
                return Ok(r);
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        // still empty: dump the page so we can see why (consent/captcha/new markup)
        if let Ok(s) = self.snapshot().await {
            println!(
                "[results] empty. url={} title={}\n{}",
                s.url,
                s.title,
                s.render()
            );
        }
        Ok(vec![])
    }

    pub async fn goto(&self, url: &str) -> Result<()> {
        self.pg()
            .await
            .goto(url)
            .await?
            .wait_for_navigation()
            .await?;
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        Ok(self.pg().await.evaluate(SNAPSHOT_JS).await?.into_value()?)
    }

    pub async fn scroll(&self, dy: i32) -> Result<()> {
        self.pg()
            .await
            .evaluate(format!("window.scrollBy(0,{dy})"))
            .await?;
        Ok(())
    }

    pub async fn eval_json(&self, js: &str) -> Result<serde_json::Value> {
        Ok(self.pg().await.evaluate(js).await?.into_value()?)
    }

    async fn by_id(&self, id: u32) -> Result<chromiumoxide::Element> {
        self.pg()
            .await
            .find_element(format!("[data-aid='{id}']"))
            .await
            .map_err(|_| anyhow!("element {id} not found; take a new snapshot"))
    }

    pub async fn click(&self, id: u32) -> Result<()> {
        let el = self.by_id(id).await?;
        el.scroll_into_view().await?;
        el.click().await?;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        Ok(())
    }

    pub async fn type_text(&self, id: u32, text: &str, submit: bool) -> Result<()> {
        let el = self.by_id(id).await?;
        el.click().await?.type_str(text).await?;
        if submit {
            el.press_key("Enter").await?;
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
        Ok(())
    }
}

pub fn resolve_browser_exe_path(name: &str) -> Option<String> {
    let exe = match resolve_app(name) {
        Some((_, exe)) => exe,
        None => name.to_string(),
    };
    let exe_with_ext = if exe.to_lowercase().ends_with(".exe") {
        exe.clone()
    } else {
        format!("{exe}.exe")
    };

    let output = std::process::Command::new("reg")
        .args([
            "query",
            &format!(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe_with_ext}"),
            "/ve",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|l| l.contains("REG_SZ"))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
}

pub fn browser_search(query: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(Key::Control, Press)?;
    enigo.key(Key::Unicode('l'), Click)?;
    enigo.key(Key::Control, Release)?;
    std::thread::sleep(std::time::Duration::from_millis(200));
    enigo.text(query)?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    enigo.key(Key::Return, Click)?;
    Ok(())
}
// ---------- moved from helper_functions ----------
pub fn open_browser_search(browser: Option<&str>, query: &str) -> Result<()> {
    let url = format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(query)
    );
    let exe = browser.map(|n| {
        resolve_app(n)
            .map(|(_, e)| e)
            .unwrap_or_else(|| n.to_string())
    });
    let mut cmd = silent_command("cmd");
    match exe {
        Some(e) => cmd.args(["/C", "start", "", "/B", &e, &url]),
        None => cmd.args(["/C", "start", "", "/B", &url]),
    };
    if cmd.spawn().is_err() {
        // fall back to the default browser
        silent_command("cmd")
            .args(["/C", "start", "", "/B", &url])
            .spawn()?;
    }
    Ok(())
}

// ---------- singleton + high-level entry points ----------
static SESSION: Lazy<Mutex<Option<(String, Arc<BrowserSession>)>>> = Lazy::new(|| Mutex::new(None));

pub async fn session(exe_path: &str) -> Result<Arc<BrowserSession>> {
    let mut g = SESSION.lock().await;
    if let Some((path, s)) = g.as_ref() {
        if path == exe_path && s.snapshot().await.is_ok() {
            return Ok(s.clone());
        }
    }
    // profile dir per browser, so profiles never clash
    let tag = std::path::Path::new(exe_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("browser");
    let dir = dirs::data_local_dir()
        .unwrap_or_default()
        .join("Arceus")
        .join(format!("profile-{tag}"));
    let s = Arc::new(BrowserSession::launch(exe_path, dir.to_str().unwrap()).await?);
    *g = Some((exe_path.to_string(), s.clone()));
    Ok(s)
}

/// "search amazon": runs in the controlled browser
pub async fn search(exe_path: &str, query: &str) -> Result<()> {
    let s = session(exe_path).await?;
    s.goto(&format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(query)
    ))
    .await
}

/// "open second link": n is 1-based
// pub async fn open_result(exe_path: &str, n: usize) -> Result<String> {
//     let s = session(exe_path).await?;
//     let r = s.results().await?;
//     let (title, url) = r
//         .get(n.saturating_sub(1))
//         .ok_or_else(|| anyhow!("only {} results on this page", r.len()))?;
//     s.goto(url).await?;
//     Ok(title.clone())
// }

pub async fn current() -> Result<Arc<BrowserSession>> {
    let g = SESSION.lock().await;
    match g.as_ref() {
        Some((_, s)) if s.snapshot().await.is_ok() => Ok(s.clone()),
        _ => Err(anyhow!("no browser open, search something first")),
    }
}

pub async fn open_result(n: usize) -> Result<String> {
    let s = current().await?;
    let r = s.results().await?;
    let (title, url) = r
        .get(n.saturating_sub(1))
        .ok_or_else(|| anyhow!("only {} results on this page", r.len()))?;
    s.goto(url).await?;
    Ok(title.clone())
}

pub async fn switch_tab(cmd: &str) -> Result<String> {
    current().await?.tab(cmd).await
}
