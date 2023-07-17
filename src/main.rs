use axum::{extract::State, response::Html, routing::get, Router};
use dashmap::DashMap;
use error::AppError;
use http::Uri;
use std::{ops::Deref, str::FromStr, sync::Arc};

mod error;

type AppResult<T> = Result<T, AppError>;

enum ArtKind {
    Twitter,
}

impl FromStr for ArtKind {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "twitter.com" => Ok(Self::Twitter),
            _ => Err("not support website".into()),
        }
    }
}

struct Art {
    url: Uri,
    kind: ArtKind,
}

impl FromStr for Art {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Uri = s.parse()?;
        let kind: ArtKind = url.authority().unwrap().host().parse()?;

        Ok(Self { url, kind })
    }
}

struct Data {
    // actual arts
    art: Vec<Art>,
}

impl Data {
    fn parse(data: &str) -> AppResult<Self> {
        let mut this = Self {
            art: Default::default(),
        };

        for entry in data.lines() {
            let art: Art = entry.parse()?;
            this.art.push(art);
        }

        Ok(this)
    }

    fn pick_random_art(&self) -> &Art {
        let no = fastrand::usize(0..self.art.len());
        &self.art[no]
    }
}

struct InternalAppState {
    // cached direct links to images
    direct_links: DashMap<Uri, String>,
    data: Data,
    http: reqwest::Client,
}

#[derive(Clone)]
struct AppState {
    internal: Arc<InternalAppState>,
}

impl AppState {
    fn new(data: Data) -> Self {
        Self {
            internal: Arc::new(InternalAppState {
                data,
                direct_links: Default::default(),
                http: reqwest::ClientBuilder::new()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap(),
            }),
        }
    }
}

impl Deref for AppState {
    type Target = InternalAppState;

    fn deref(&self) -> &Self::Target {
        &self.internal
    }
}

#[tokio::main]
async fn main() {
    let arts = std::fs::read_to_string("arts.txt").unwrap();
    let data = AppState::new(Data::parse(&arts).unwrap());
    let app = Router::new().route("/", get(show_art)).with_state(data);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn show_art(state: State<AppState>) -> AppResult<Html<String>> {
    let art = state.data.pick_random_art();
    if let Some(image_link) = state.direct_links.get(&art.url) {
        Ok(render_page(art, &image_link))
    } else {
        let image_link = match art.kind {
            ArtKind::Twitter => fetch_twitter_image_link(&state.http, &art.url).await?,
        };
        let page = render_page(art, &image_link);
        state.direct_links.insert(art.url.clone(), image_link);
        Ok(page)
    }
}

fn render_page(art: &Art, image_link: &str) -> Html<String> {
    let body_style =
        "margin: 0px; background: #0e0e0e; height: 100vh; width: 100vw; display: flex;";
    let img_style = "display: block; margin: auto; max-height: 100vh; max-width: 100vw;";
    let about_style = "position: absolute; bottom: 0; font-size: 0.75em; color: #ffffff; background-color: #0e0e0eaa;";
    let content = maud::html! {
        (maud::DOCTYPE)
        head {
            meta charset="utf8";
            meta property="og:image" content=(image_link);
            title { "random limbussy art" }
        }
        body style=(body_style) {
            img style=(img_style) src=(image_link);
            a style=(format!("{about_style} left: 0;")) href=(art.url) target="_blank" {
                "source: " (art.url)
            }
            a style=(format!("{about_style} right: 0;")) href="https://gaze.systems" target="_blank" {
                "website made by dusk"
            }
        }
    };
    Html(content.into_string())
}

async fn fetch_twitter_image_link(http: &reqwest::Client, url: &Uri) -> AppResult<String> {
    let fxurl = Uri::builder()
        .scheme("https")
        .authority("d.fxtwitter.com")
        .path_and_query(url.path_and_query().unwrap().clone())
        .build()?
        .to_string();
    let req = http.get(fxurl).build()?;
    let resp = http.execute(req).await?.error_for_status()?;
    let link = resp
        .headers()
        .get(http::header::LOCATION)
        .unwrap()
        .to_str()?
        .to_owned();
    Ok(link)
}
