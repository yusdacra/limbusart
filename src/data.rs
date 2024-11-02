use std::{collections::HashMap, str::FromStr};

use http::Uri;

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub(crate) enum ArtKind {
    Twitter,
    Safebooru,
}

impl FromStr for ArtKind {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "twitter.com" => Ok(Self::Twitter),
            "safebooru.org" => Ok(Self::Safebooru),
            _ => Err("not support website".into()),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Art {
    pub(crate) url: Uri,
    pub(crate) kind: ArtKind,
}

impl FromStr for Art {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Uri = s.parse()?;
        let kind: ArtKind = url.authority().unwrap().host().parse()?;

        Ok(Self { url, kind })
    }
}

pub(crate) struct Data {
    // actual arts
    art: Vec<Art>,
    art_indices: HashMap<Uri, usize>,
}

impl Data {
    pub(crate) fn parse(data: &str) -> AppResult<Self> {
        let mut this = Self {
            art: Default::default(),
            art_indices: Default::default(),
        };

        for entry in data.lines() {
            let art: Art = entry.parse()?;
            this.art_indices.insert(art.url.clone(), this.art.len());
            this.art.push(art);
        }

        Ok(this)
    }

    pub(crate) fn pick_random_art(&self) -> &Art {
        let no = fastrand::usize(0..self.art.len());
        &self.art[no]
    }

    pub(crate) fn reload(&mut self, data: &str) -> AppResult<()> {
        for entry in data.lines() {
            let art: Art = entry.parse()?;
            if !self.art_indices.contains_key(&art.url) {
                self.art_indices.insert(art.url.clone(), self.art.len());
                self.art.push(art);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct FetchedLink {
    pub(crate) image_url: String,
    pub(crate) new_source: Option<Uri>,
}
