//! The music library: a fictional catalogue of artists, albums, tracks and playlists,
//! generated once at start-up from a fixed seed.
//!
//! **Why generated.** The player needs a library large enough to show that the track list
//! scrolls, searches and sorts without effort (about 500 songs), with names that read like a
//! real collection, and no real artists. Artist and album names are hand-written; song titles
//! are composed from word lists by a seeded [`Rng`], so every run (and every scripted
//! recording) sees the same library.
//!
//! **How it is used.** The catalogue is immutable and lives in a `static` ([`catalog`]); the
//! UI never copies it. Lists hold track *ids* (`u32`) and look the rest up here, so a keyed
//! row or a queue entry is a few bytes, and a filter or sort over the whole library is a
//! `Vec<u32>` built by a memo. Search runs against a lower-case haystack prepared per track.

use std::sync::OnceLock;

use crate::util::Rng;

/// A musical style. It names the genre in the UI and shapes the simulated sound
/// (tempo, spectral balance, which instruments play: see `audio::Profile`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Genre {
    Synthwave,
    IndiePop,
    Jazz,
    Soul,
    Ambient,
    House,
    Folk,
    HipHop,
    Rock,
}

impl Genre {
    /// The genre's display name.
    pub const fn name(self) -> &'static str {
        match self {
            Genre::Synthwave => "Synthwave",
            Genre::IndiePop => "Indie Pop",
            Genre::Jazz => "Jazz",
            Genre::Soul => "Soul",
            Genre::Ambient => "Ambient",
            Genre::House => "House",
            Genre::Folk => "Folk",
            Genre::HipHop => "Hip-Hop",
            Genre::Rock => "Rock",
        }
    }
}

/// How an album cover is drawn (see `art`): a motif and two hues, chosen per album.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motif {
    /// Concentric rings around an off-centre point.
    Rings,
    /// A sun setting behind horizontal bands.
    Sunset,
    /// Diagonal colour bands.
    Bands,
    /// Planets on orbits.
    Orbits,
    /// Layered mountain ridges.
    Peaks,
    /// Stacked sine waves.
    Waves,
    /// A perspective grid under a glowing horizon.
    Grid,
    /// Overlapping translucent circles.
    Bloom,
}

const MOTIFS: [Motif; 8] = [
    Motif::Rings,
    Motif::Sunset,
    Motif::Bands,
    Motif::Orbits,
    Motif::Peaks,
    Motif::Waves,
    Motif::Grid,
    Motif::Bloom,
];

/// An artist.
#[derive(Debug)]
pub struct Artist {
    pub name: &'static str,
    pub genre: Genre,
    /// Their albums, oldest first.
    pub albums: Vec<u32>,
}

/// An album and the recipe of its cover.
#[derive(Debug)]
pub struct Album {
    pub id: u32,
    pub title: &'static str,
    pub artist: u32,
    pub year: u16,
    pub motif: Motif,
    /// Hue of the cover's dominant colour, in degrees.
    pub hue: u16,
    /// Hue of the cover's second colour, in degrees.
    pub hue2: u16,
    /// Varies the motif's geometry between covers that share it.
    pub seed: u32,
    /// Its tracks in album order.
    pub tracks: Vec<u32>,
}

/// A song.
#[derive(Debug)]
pub struct Track {
    pub id: u32,
    pub title: String,
    pub album: u32,
    pub artist: u32,
    /// Position on its album, from 1.
    pub number: u8,
    /// Length in seconds.
    pub secs: u32,
    /// Tempo in beats per minute.
    pub bpm: u16,
    /// Seeds the simulated audio (melody, arrangement).
    pub seed: u64,
    /// `title artist album`, lower case: what the search box matches against.
    pub haystack: String,
}

/// A playlist: a named, curated selection of tracks.
#[derive(Debug)]
pub struct Playlist {
    pub name: &'static str,
    pub blurb: &'static str,
    /// Hue of its tile.
    pub hue: u16,
    pub tracks: Vec<u32>,
}

/// The whole library.
#[derive(Debug)]
pub struct Catalog {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
}

impl Catalog {
    /// The track with `id` (ids are indices, so this is a direct read).
    pub fn track(&self, id: u32) -> &Track {
        &self.tracks[id as usize]
    }

    /// The album with `id`.
    pub fn album(&self, id: u32) -> &Album {
        &self.albums[id as usize]
    }

    /// The artist with `id`.
    pub fn artist(&self, id: u32) -> &Artist {
        &self.artists[id as usize]
    }

    /// The artist's name of track `id`.
    pub fn artist_of(&self, id: u32) -> &'static str {
        self.artist(self.track(id).artist).name
    }

    /// The album title of track `id`.
    pub fn album_of(&self, id: u32) -> &'static str {
        self.album(self.track(id).album).title
    }

    /// The genre of track `id`.
    pub fn genre_of(&self, id: u32) -> Genre {
        self.artist(self.track(id).artist).genre
    }

    /// The total length of `ids`, in seconds.
    pub fn total_secs(&self, ids: &[u32]) -> u32 {
        ids.iter().map(|&id| self.track(id).secs).sum()
    }
}

/// The library, built on first use.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(build)
}

// ---- Source material ------------------------------------------------------------------------

/// Hand-written, fictional artists and their styles.
const ARTISTS: [(&str, Genre); 36] = [
    ("Neon Cartography", Genre::Synthwave),
    ("Mara Quill", Genre::IndiePop),
    ("The Lantern Hours", Genre::Rock),
    ("Velvet Static", Genre::Synthwave),
    ("Odessa Fade", Genre::Soul),
    ("Juniper Vale", Genre::Folk),
    ("Satellite Hearts", Genre::IndiePop),
    ("Low Orbit Choir", Genre::Ambient),
    ("Emmett Sorel Quartet", Genre::Jazz),
    ("Paper Moons Club", Genre::House),
    ("Coral Tapes", Genre::Synthwave),
    ("Isla Marrow", Genre::Soul),
    ("The Quiet Engines", Genre::Rock),
    ("Saffron Drive", Genre::IndiePop),
    ("Hollow Pines", Genre::Folk),
    ("Theo Lindqvist Trio", Genre::Jazz),
    ("Glasshouse Radio", Genre::House),
    ("Amber Lowe", Genre::Soul),
    ("Midnight Cassette", Genre::Synthwave),
    ("Rue Delphine", Genre::Jazz),
    ("Static Bloom", Genre::Ambient),
    ("Northern Ferns", Genre::Folk),
    ("Cobalt Avenue", Genre::HipHop),
    ("Sienna Hart", Genre::IndiePop),
    ("Polar Disco", Genre::House),
    ("Wren & Willow", Genre::Folk),
    ("Monday Arcade", Genre::Rock),
    ("Kasimir Blue", Genre::HipHop),
    ("Halcyon Drift", Genre::Ambient),
    ("Lumen Park", Genre::Synthwave),
    ("Ivory Lagoon", Genre::Ambient),
    ("Marigold Social", Genre::IndiePop),
    ("Ezra Calloway", Genre::HipHop),
    ("Tidepool Assembly", Genre::House),
    ("Garnet & Gold", Genre::Soul),
    ("The Overpass Kids", Genre::Rock),
];

/// Hand-written album titles; album `i` belongs to artist `i % 36`.
const ALBUMS: [&str; 48] = [
    "Glass Meridian",
    "Small Hours",
    "Lights Over Harbor Street",
    "Chrome Lullabies",
    "Honey & Smoke",
    "Fieldnotes",
    "Teenage Satellites",
    "Weightless",
    "Blue Arithmetic",
    "Dancefloor Weather",
    "Rewind Season",
    "Golden Hour Letters",
    "Machine Heart",
    "Postcards from June",
    "Cedar & Ash",
    "Late Set at the Marlowe",
    "Signal / Noise",
    "Velvet Evidence",
    "Nightdrive 1987",
    "Cafe Lumiere",
    "Slow Light",
    "North of Quiet",
    "Concrete Psalms",
    "Paper Planets",
    "Ice Palace Disco",
    "Hearth Songs",
    "Static on the Line",
    "Blue Hour Tapes",
    "Tidal Memory",
    "Afterglow Boulevard",
    "Seafoam",
    "Pastel Riot",
    "Long Way Home",
    "Pool Party at Dusk",
    "Satin Hours",
    "Overpass Anthems",
    "Midnight Geometry",
    "The Second Spring",
    "Electric Orchard",
    "Neon Rain",
    "Bittersweet Engine",
    "Wildflower Radio",
    "Planetarium",
    "Quiet Fire",
    "After Hours Club",
    "Silver Lining Sessions",
    "Daydream Index",
    "Low Tide Choir",
];

const ADJECTIVES: [&str; 40] = [
    "Golden", "Quiet", "Electric", "Velvet", "Silver", "Midnight", "Paper", "Broken", "Endless", "Neon",
    "Hollow", "Crimson", "Gentle", "Restless", "Distant", "Burning", "Faded", "Wild", "Little", "Open",
    "Northern", "Slow", "Bright", "Heavy", "Hidden", "Painted", "Lucid", "Tender", "Lonely", "Second",
    "Sunlit", "Phantom", "Glass", "Summer", "Winter", "Coastal", "Easy", "Final", "Rosy", "Analog",
];

const NOUNS: [&str; 48] = [
    "Heart",
    "Static",
    "Satellites",
    "Afterglow",
    "Rivers",
    "Signals",
    "Weather",
    "Avenue",
    "Lights",
    "Horizon",
    "Echoes",
    "Gardens",
    "Mirrors",
    "Tides",
    "Ghosts",
    "Season",
    "Highway",
    "Daydream",
    "Embers",
    "Harbor",
    "Skyline",
    "Fever",
    "Orchard",
    "Comets",
    "Letters",
    "Cassette",
    "Paradise",
    "Shadows",
    "Thunder",
    "Velocity",
    "Pilgrims",
    "Lanterns",
    "Gravity",
    "Marigolds",
    "Telephone",
    "Undertow",
    "Wildfire",
    "Kingdom",
    "Monsoon",
    "Carousel",
    "Silhouette",
    "Frequencies",
    "Overture",
    "Postcards",
    "Sundown",
    "Rooftops",
    "Constellations",
    "Moonlight",
];

const VERBS: [&str; 20] = [
    "Chasing",
    "Dancing",
    "Waiting",
    "Falling",
    "Running",
    "Holding",
    "Calling",
    "Dreaming",
    "Drifting",
    "Burning",
    "Leaving",
    "Finding",
    "Singing",
    "Breaking",
    "Turning",
    "Counting",
    "Wearing",
    "Losing",
    "Painting",
    "Following",
];

const PLACES: [&str; 20] = [
    "Lisbon",
    "Kyoto",
    "the Valley",
    "Harbor Street",
    "the Coast",
    "Avenue B",
    "Marseille",
    "the North",
    "Saturn",
    "the Pier",
    "Brooklyn",
    "Tangier",
    "the Arcade",
    "Reykjavik",
    "the Motel",
    "Paradise Road",
    "Oslo",
    "the Lake",
    "Union Square",
    "the Old Town",
];

const VERSIONS: [&str; 6] = [
    "Night Version",
    "Reprise",
    "Live at the Marlowe",
    "Demo",
    "Extended Mix",
    "Acoustic",
];

/// A song title in one of several common shapes.
fn song_title(rng: &mut Rng) -> String {
    let adj = *rng.pick(&ADJECTIVES);
    let noun = *rng.pick(&NOUNS);
    match rng.range(0, 11) {
        0..=2 => format!("{adj} {noun}"),
        3 => noun.to_string(),
        4 => format!("{} the {noun}", rng.pick(&VERBS)),
        5 => format!("{} in {}", rng.pick(&VERBS), rng.pick(&PLACES)),
        6 => format!("{noun} & {}", rng.pick(&NOUNS)),
        7 => format!("Last {noun} in {}", rng.pick(&PLACES)),
        8 => format!("{adj} {noun} ({})", rng.pick(&VERSIONS)),
        9 => format!("The {noun} of {}", rng.pick(&PLACES)),
        10 => format!("{} {adj}", rng.pick(&VERBS)),
        _ => format!("Nights in {}", rng.pick(&PLACES)),
    }
}

/// A typical tempo for the genre.
fn tempo(genre: Genre, rng: &mut Rng) -> u16 {
    let (lo, hi) = match genre {
        Genre::Synthwave => (96, 118),
        Genre::IndiePop => (104, 128),
        Genre::Jazz => (84, 140),
        Genre::Soul => (72, 96),
        Genre::Ambient => (60, 80),
        Genre::House => (120, 126),
        Genre::Folk => (78, 112),
        Genre::HipHop => (84, 96),
        Genre::Rock => (110, 150),
    };
    rng.range(lo, hi) as u16
}

fn build() -> Catalog {
    let mut rng = Rng::new(0x7_7714E);
    let mut artists: Vec<Artist> = ARTISTS
        .iter()
        .map(|&(name, genre)| Artist {
            name,
            genre,
            albums: Vec::new(),
        })
        .collect();
    let mut albums = Vec::with_capacity(ALBUMS.len());
    let mut tracks = Vec::new();
    for (i, &title) in ALBUMS.iter().enumerate() {
        let id = i as u32;
        let artist = (i % ARTISTS.len()) as u32;
        let genre = ARTISTS[artist as usize].1;
        let hue = rng.range(0, 359) as u16;
        let mut album = Album {
            id,
            title,
            artist,
            // A second album is always newer than the first.
            year: if i < ARTISTS.len() {
                2004 + rng.range(0, 13)
            } else {
                2018 + rng.range(0, 8)
            } as u16,
            motif: MOTIFS[i % MOTIFS.len()],
            hue,
            hue2: (hue + 40 + rng.range(0, 100) as u16) % 360,
            seed: rng.range(0, u32::MAX - 1),
            tracks: Vec::new(),
        };
        for number in 1..=rng.range(8, 13) as u8 {
            let tid = tracks.len() as u32;
            let title = song_title(&mut rng);
            let haystack = format!("{title} {} {}", ARTISTS[artist as usize].0, album.title).to_lowercase();
            tracks.push(Track {
                id: tid,
                title,
                album: id,
                artist,
                number,
                secs: rng.range(148, 372),
                bpm: tempo(genre, &mut rng),
                seed: rng.next_u64(),
                haystack,
            });
            album.tracks.push(tid);
        }
        artists[artist as usize].albums.push(id);
        albums.push(album);
    }
    let playlists = playlists(&tracks, &artists, &mut rng);
    Catalog {
        artists,
        albums,
        tracks,
        playlists,
    }
}

/// Curated playlists: each picks tracks by mood (genre and tempo), in a seeded order.
fn playlists(tracks: &[Track], artists: &[Artist], rng: &mut Rng) -> Vec<Playlist> {
    type Rule = fn(Genre, u16) -> bool;
    let defs: [(&str, &str, u16, Rule); 6] = [
        (
            "Late Night Drive",
            "Synths, tail lights and an empty highway.",
            280,
            |g, _| matches!(g, Genre::Synthwave | Genre::House),
        ),
        (
            "Deep Focus",
            "Unhurried textures for long stretches of work.",
            200,
            |g, _| matches!(g, Genre::Ambient | Genre::Jazz),
        ),
        (
            "Sunday Morning",
            "Coffee, sunlight and acoustic guitars.",
            38,
            |g, bpm| matches!(g, Genre::Folk | Genre::Soul) && bpm < 100,
        ),
        (
            "Running Club",
            "Steady tempo above 120 BPM, no slow songs.",
            8,
            |_, bpm| bpm >= 120,
        ),
        ("Smoke & Velvet", "Soul and jazz for after dark.", 330, |g, _| {
            matches!(g, Genre::Soul | Genre::Jazz)
        }),
        (
            "Fresh Finds",
            "Indie pop and rock you have not heard yet.",
            150,
            |g, _| matches!(g, Genre::IndiePop | Genre::Rock | Genre::HipHop),
        ),
    ];
    defs.iter()
        .map(|&(name, blurb, hue, rule)| {
            let mut ids: Vec<u32> = tracks
                .iter()
                .filter(|t| rule(artists[t.artist as usize].genre, t.bpm))
                .map(|t| t.id)
                .collect();
            // A curator's order (not album order), and at most 40 songs.
            for i in (1..ids.len()).rev() {
                ids.swap(i, rng.range(0, i as u32) as usize);
            }
            ids.truncate(40);
            Playlist {
                name,
                blurb,
                hue,
                tracks: ids,
            }
        })
        .collect()
}
