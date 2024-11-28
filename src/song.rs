use crate::{data::Database, require};
use base64::{prelude::BASE64_STANDARD, Engine};
use lofty::{file::TaggedFileExt, probe::Probe, tag::Accessor};
use log::debug;
use musicbrainz_rs_nova::{
    entity::{
        recording::{Recording, RecordingSearchQuery},
        release_group::{ReleaseGroup, ReleaseGroupSearchQuery},
    },
    Browse, Search,
};
use petname::Generator;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cell::{LazyCell, OnceCell},
    fmt::format,
    fs::{self, File},
    io::{BufRead, BufReader, Cursor, Read, Seek, SeekFrom},
    iter::Once,
    ops::Range,
    sync::{LazyLock, Mutex},
    thread,
    time::Duration,
};
use tiny_http::{Header, Request, Response, ResponseBox};

#[derive(Deserialize)]
struct ListDataRequest {}
#[derive(Deserialize, Serialize)]
struct Data {
    id: u32,
    time: u64,
    value: String,
}

pub(crate) fn list(db: &Database, req: &mut Request) -> ResponseBox {
    let r: ListDataRequest = crate::try_json!(req);

    let json = db.get_all_json();

    Response::from_string(json)
        .with_status_code(200)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
        .boxed()
}

pub(crate) fn get_audio_page(db: &Database, req: &mut Request) -> ResponseBox {
    let url = req.url();
    let mut components = url.split('/');
    components.next();
    components.next();
    let title = components.next();
    let artist = components.next();

    match (title, artist) {
        (Some(title), artist) => {
            let Some(song) = db.get_song_by_title_and_artist(
                &url_escape::decode(title),
                &url_escape::decode(artist.unwrap_or("")),
            ) else {
                return Response::from_string("").with_status_code(404).boxed();
            };

            let html = get_audio_page_html(song);

            return Response::from_string(html)
                .with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]).unwrap(),
                )
                .with_status_code(200)
                .boxed();
        }
        (None, _) => {
            return Response::from_string("").with_status_code(404).boxed();
        }
    }
}

fn get_audio_page_html(song: crate::data::SongEntry<'_>) -> String {
    format!(
        "<html><audio controls src=\"/data/{}\"></audio>",
        song.song_path
    )
}

enum HttpRange {
    Inclusive { start: u64, end: u64 },
    Open { start: u64 },
    Negative { value: i64 },
}

fn parse_range_header(header: &Header) -> anyhow::Result<HttpRange> {
    let value = header.value.as_str();

    let (_, range) = value
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("invalid format"))?;

    let (from, to) = range
        .split_once('-')
        .ok_or_else(|| anyhow::anyhow!("invalid format"))?;

    if to.is_empty() {
        let from = from.parse()?;

        Ok(HttpRange::Open { start: from })
    } else {
        let from = from.parse()?;
        let to = to.parse()?;

        Ok(HttpRange::Inclusive {
            start: from,
            end: to,
        })
    }
}

pub(crate) fn get_audio_data(db: &Database, req: &mut Request) -> ResponseBox {
    let Some(header) = req
        .headers()
        .iter()
        .find(|h| h.field == "x-real-ip".parse().unwrap())
    else {
        log::warn!("No IP header found");
        return Response::from_string("").with_status_code(400).boxed();
    };

    let range_header = req
        .headers()
        .iter()
        .find(|h| h.field == "Range".parse().unwrap())
        .map(|h| parse_range_header(h));

    let ip = header.value.as_str();
    if !db.is_allowed(&ip) {
        debug!("IP {ip} is not allowed");
        return Response::from_string("").with_status_code(403).boxed();
    }

    debug!("Allowed {ip}");

    let url = req.url();
    let mut components = url.split('/');
    components.next();
    components.next();
    let Some(file) = components.next() else {
        return Response::from_string("").with_status_code(404).boxed();
    };

    let Ok(mut file) = File::open(format!("./songs/{}", file)) else {
        return Response::from_string("").with_status_code(404).boxed();
    };

    let file_size = file.metadata().ok().map(|v| v.len() as usize);

    if let Some(rh) = range_header {
        let Ok(range) = rh else {
            return Response::from_string("").with_status_code(400).boxed();
        };
        let Some(file_size) = file_size else {
            return Response::from_string("").with_status_code(400).boxed();
        };

        let mut headers = vec![];

        match range {
            HttpRange::Inclusive { start, end } => {
                let len = end - start;
                let len_value = format!("{len}");
                let content_range = format!("bytes {start}-{end}/{file_size}");
                headers.push(
                    Header::from_bytes(&b"content-length"[..], len_value.as_bytes()).unwrap(),
                );
                headers.push(
                    Header::from_bytes(&b"content-range"[..], content_range.as_bytes()).unwrap(),
                );

                let Ok(_) = file.seek(SeekFrom::Start(start)) else {
                    return Response::from_string("").with_status_code(500).boxed();
                };

                let reader = BufReader::new(file);
                return Response::new(
                    tiny_http::StatusCode(206),
                    headers,
                    reader.take(len),
                    None,
                    None,
                )
                .boxed();
            }
            HttpRange::Open { start } => {
                let len = file_size as u64 - start;
                let len_value = format!("{len}");
                let content_range = format!("bytes {start}-{file_size}/{file_size}");
                headers.push(
                    Header::from_bytes(&b"content-length"[..], len_value.as_bytes()).unwrap(),
                );
                headers.push(
                    Header::from_bytes(&b"content-range"[..], content_range.as_bytes()).unwrap(),
                );

                let Ok(_) = file.seek(SeekFrom::Start(start)) else {
                    return Response::from_string("").with_status_code(500).boxed();
                };

                let reader = BufReader::new(file);
                return Response::new(tiny_http::StatusCode(206), headers, reader, None, None)
                    .boxed();
            }
            HttpRange::Negative { value } => todo!(),
        }

        todo!()
    } else {
        Response::new(
            tiny_http::StatusCode(200),
            vec![Header::from_bytes(&b"accept-ranges"[..], &b"bytes"[..]).unwrap()],
            file,
            file_size,
            None,
        )
        .boxed()
    }
}

#[derive(Deserialize, Serialize)]
struct ProbeSongRequest {
    song_data_base64: String,
}

#[derive(Default, Serialize)]
struct ProbeSongResponse {
    title: Option<String>,
    artists: Vec<String>,
    album: Option<String>,
    genres: Vec<String>,
}

fn get_metadata(song_data_base64: String) -> anyhow::Result<ProbeSongResponse> {
    let data = BASE64_STANDARD.decode(song_data_base64)?;

    let probe = Probe::new(Cursor::new(data)).guess_file_type()?;
    let file = probe.read()?;
    let t = file.primary_tag();
    let Some(tag) = t else {
        return Err(anyhow::anyhow!("No tags found"));
    };

    let title = tag.title().ok_or(anyhow::anyhow!("No title found"))?;
    let artist = tag.artist();
    let album = tag.album();

    let recordingz = get_musicbrainz_metadata(&title, artist.clone(), album.clone());
    if let Ok((rec, rg)) = recordingz {
        let album = if let Some(rg) = rg {
            Some(rg.title)
        } else {
            album.map(|a| a.to_string()).or(rec
                .releases
                .and_then(|r| r.into_iter().next().map(|r| r.title)))
        };

        let mut artists = Vec::new();
        if let Some(artist) = artist {
            artists.push(artist.to_string());
        }

        return Ok(ProbeSongResponse {
            title: Some(rec.title),
            artists: rec
                .artist_credit
                .map(|a| a.into_iter().map(|c| c.artist.name).collect())
                .unwrap_or(artists),
            album: album,
            genres: rec
                .genres
                .map(|g| g.into_iter().map(|g| g.name).collect())
                .unwrap_or(vec![]),
        });
    }

    return Ok(ProbeSongResponse {
        title: Some(title.into_owned()),
        album: album.map(|a| a.into_owned()),
        artists: artist.map(|a| vec![a.into_owned()]).unwrap_or(vec![]),
        ..Default::default()
    });
}

static BRAINZ_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn get_musicbrainz_metadata<'a>(
    title: &str,
    artist: Option<Cow<'a, str>>,
    album: Option<Cow<'a, str>>,
) -> anyhow::Result<(Recording, Option<ReleaseGroup>)> {
    let guard = BRAINZ_MUTEX.lock().unwrap();

    println!(
        "Fetching MusicBrainz data using title: {title}, artist: {artist:?}, album: {album:?}"
    );

    thread::sleep(Duration::from_millis(1500));

    match (artist, album) {
        (Some(artist), Some(album)) => {
            let q: String = ReleaseGroupSearchQuery::query_builder()
                .release_group(&album)
                .and()
                .artist(&artist)
                .build();

            let release_group = ReleaseGroup::search(q)
                .execute()?
                .entities
                .into_iter()
                .next();

            if let Some(rg) = &release_group {
                println!("Found release group: {}", rg.title);

                thread::sleep(Duration::from_millis(1500));

                let q: String = RecordingSearchQuery::query_builder()
                    .recording(title)
                    .and()
                    .artist(&artist)
                    .and()
                    .rgid(&rg.id)
                    .build();

                let recording: Option<Recording> = Recording::search(q)
                    .with_genres()
                    .execute()?
                    .entities
                    .into_iter()
                    .next();
                let recording = recording.ok_or(anyhow::anyhow!("Failed to find recording"))?;

                return Ok((recording, release_group));
            }

            println!("Retrying without album");

            // Try again without album
            drop(guard);
            return get_musicbrainz_metadata(title, Some(artist), None);
        }
        (Some(artist), _) => {
            let q: String = RecordingSearchQuery::query_builder()
                .recording(title)
                .and()
                .artist(&artist)
                .build();
            let recording = Recording::search(q)
                .with_genres()
                .execute()?
                .entities
                .into_iter()
                .next();

            let recording = recording.ok_or(anyhow::anyhow!("Failed to find recording"))?;

            return Ok((recording, None));
        }
        _ => {
            let q: String = RecordingSearchQuery::query_builder()
                .recording(title)
                .build();
            let recording = Recording::search(q)
                .with_genres()
                .execute()?
                .entities
                .into_iter()
                .next();
            let recording = recording.ok_or(anyhow::anyhow!("Failed to find recording"))?;

            return Ok((recording, None));
        }
    }
}

pub(crate) fn probe(db: &Database, req: &mut Request) -> ResponseBox {
    let username = crate::try_auth!(db, req);
    let r: ProbeSongRequest = crate::try_json!(req);

    require!(r.song_data_base64.len() < 1024 * 1024 * 130);

    match get_metadata(r.song_data_base64) {
        Ok(md) => {
            return crate::to_json!(&md);
        }
        Err(e) => {
            return Response::from_string(format!("{e:?}"))
                .with_status_code(400)
                .boxed();
        }
    }
}

#[derive(Deserialize)]
struct AddSongRequest {
    song_data_filename: String,
    song_data_base64: String,
    title: String,
    artists: Vec<String>,
    album: String,
    genres: Vec<String>,
}

#[derive(Serialize)]
struct AddSongResponse {}

pub(crate) fn add(db: &Database, req: &mut Request) -> ResponseBox {
    let username = crate::try_auth!(db, req);
    let r: AddSongRequest = crate::try_json!(req);

    let extension = r.song_data_filename.split('.').last().unwrap();

    require!(r.song_data_base64.len() < 1024 * 1024 * 130);

    let data = BASE64_STANDARD.decode(r.song_data_base64).unwrap();
    let mut rng = rand::thread_rng();
    let name = petname::Petnames::small()
        .generate(&mut rng, 7, "-")
        .expect("no names");
    let path = format!("./songs/{}.{}", name, extension);
    //let path = format!("./{}", r.song_file_name);
    fs::write(&path, data).unwrap();

    db.add_song(&crate::data::SongEntry {
        title: r.title.into(),
        album: r.album.into(),
        artists: r.artists.into_iter().map(|g| g.into()).collect(),
        genres: r.genres.into_iter().map(|g| g.into()).collect(),
        song_path: format!("{}.{}", name, extension).into(),
    });

    Response::from_string("{}").with_status_code(200).boxed()
}
