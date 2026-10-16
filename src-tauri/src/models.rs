use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// A file in the user's local music folders (or an offline download).
    Local,
    Soundcloud,
    Ytmusic,
    Youtube,
    Spotify,
}

impl Platform {
    pub const ALL: [Platform; 5] = [Platform::Local, Platform::Ytmusic, Platform::Soundcloud, Platform::Youtube, Platform::Spotify];

    pub fn label(self) -> &'static str {
        match self {
            Platform::Local => "Local files",
            Platform::Soundcloud => "SoundCloud",
            Platform::Ytmusic => "YouTube Music",
            Platform::Youtube => "YouTube",
            Platform::Spotify => "Spotify",
        }
    }

    pub fn from_url(url: &str) -> Option<Platform> {
        let u = url.to_lowercase();
        if !u.starts_with("http") && !u.starts_with("spotify:") {
            Some(Platform::Local)
        } else if u.contains("soundcloud.com") || u.contains("snd.sc") {
            Some(Platform::Soundcloud)
        } else if u.contains("music.youtube.com") {
            Some(Platform::Ytmusic)
        } else if u.contains("youtube.com") || u.contains("youtu.be") {
            Some(Platform::Youtube)
        } else if u.contains("spotify.com") || u.starts_with("spotify:") {
            Some(Platform::Spotify)
        } else {
            None
        }
    }
}

/// One concrete, playable (or resolvable) upload of a song on one platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackVersion {
    pub platform: Platform,
    pub url: String,
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// SoundCloud Go+ tracks only expose a 30 s preview.
    #[serde(default)]
    pub preview: bool,
    /// Metadata is a placeholder (e.g. SoundCloud set entries only carry a URL);
    /// it gets filled in from yt-dlp when the track is first played.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// Uploader/artist page on the same platform (SoundCloud user, YouTube channel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// Rank inside its own platform's result list (0 = top). Not persisted meaningfully.
    #[serde(default, skip_serializing)]
    pub rank: usize,
}

/// One logical song, possibly available on several platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    pub versions: Vec<TrackVersion>,
    #[serde(default)]
    pub score: f64,
}

impl Track {
    pub fn from_version(v: TrackVersion) -> Track {
        Track {
            key: crate::search::track_key(&v.artist, &v.title),
            title: crate::search::strip_noise(&v.title),
            artist: v.artist.clone(),
            duration: v.duration,
            thumbnail: v.thumbnail.clone(),
            versions: vec![v],
            score: 0.0,
        }
    }

    pub fn has(&self, p: Platform) -> bool {
        self.versions.iter().any(|v| v.platform == p)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePlaylist {
    pub title: String,
    pub url: String,
    pub platform: Option<Platform>,
    pub tracks: Vec<Track>,
}
