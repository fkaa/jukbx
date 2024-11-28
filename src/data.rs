use std::{borrow::Cow, fs::{self, File}, io::{BufReader, BufWriter}, sync::{Arc, RwLock}};

use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct SongEntry<'a> {
    pub title: Cow<'a, str>,
    pub album: Cow<'a, str>,
    pub artists: Vec<Cow<'a, str>>,
    pub genres: Vec<Cow<'a, str>>,
    pub song_path: Cow<'a, str>,
}

#[derive(Clone)]
pub struct Database {
    songs: Arc<RwLock<SongDatabase>>,
    passwords: Arc<RwLock<PasswordDatabase>>,
    whitelist: Arc<RwLock<WhitelistDatabase>>,
}

impl Database {
    pub fn open(song_path: String, password_path: String, whitelist_path: String) -> Self {
        Database {
            songs: Arc::new(RwLock::new(SongDatabase::new(song_path))),
            passwords: Arc::new(RwLock::new(PasswordDatabase::new(password_path))),
            whitelist: Arc::new(RwLock::new(WhitelistDatabase::new(whitelist_path))) 
        }
    }

    pub fn is_allowed(&self, ip: &str) -> bool {
        let inner = self.whitelist.read().unwrap();
        inner.is_allowed(ip)
    }

    pub fn get_song_by_title_and_artist<'a>(&self, title: &str, artist: &str) -> Option<SongEntry<'static>> {
        let inner = self.songs.read().unwrap();
        inner.get_song_by_title_and_artist(title, artist)
    }

    pub fn get_all_json(&self) -> String {
        let inner = self.songs.read().unwrap();
        inner.get_all_json()
    }

    pub fn add_song(&self, song: &SongEntry) {
        let mut inner = self.songs.write().unwrap();
        inner.add_song(song);
    }
    
    pub(crate) fn add_user(&self, user: &str, base64_pass: &str) {
        let mut inner = self.passwords.write().unwrap();
        inner.add_user(user, base64_pass);
    }

    pub(crate) fn get_user(&self, user: &str, password: &str) -> Option<String> {
        let inner = self.passwords.read().unwrap();
        inner.get_user(user, password)
    }
    
    pub(crate) fn update_user(&self, user: &str, base64_pass: &str) {
        let mut inner = self.passwords.write().unwrap();
        inner.update_user(user, base64_pass);
    }
}

struct WhitelistDatabase {
    path: String,
}
impl WhitelistDatabase {
    pub fn new(path: String) -> Self {
        WhitelistDatabase { path }
    }
    pub fn is_allowed(&self, ip: &str) -> bool {
        let mut db = self.open_database_read();

        for r in db.records() {
            let Ok(r) = r else {
                return false;
            };
            let csv_ip: &str = r.get(0).unwrap();

            if csv_ip == ip {
                return true;
            }
        }

        false
    }
    fn open_database_read(&self) -> csv::Reader<BufReader<File>> {
        let mut rdr = csv::ReaderBuilder::new().from_reader(BufReader::new(File::open(&self.path).unwrap()));
        rdr
    }
    fn open_database_write(&mut self) -> csv::Writer<BufWriter<File>> {
        let mut rdr = csv::WriterBuilder::new().from_writer(BufWriter::new(File::create(&self.path).unwrap()));
        rdr
    }
    fn open_temp_database_write(&mut self) -> csv::Writer<BufWriter<File>> {
        let mut rdr = csv::WriterBuilder::new().from_writer(BufWriter::new(File::create(&format!("{}.tmp", self.path)).unwrap()));
        rdr
    }
    fn copy_temp_database(&mut self) {
        fs::rename(&self.path, &format!("{}.bak", self.path)).unwrap();
        fs::rename(&format!("{}.tmp", self.path), &self.path).unwrap();
    }
    
}

struct PasswordDatabase {
    path: String,
}
impl PasswordDatabase {
    pub fn new(path: String) -> Self {
        PasswordDatabase { path }
    }
    pub fn get_user(&self, user: &str, passowrd: &str) -> Option<String> {
        let mut db = self.open_database_read();

        for r in db.records() {
            let Ok(r) = r else {
                return None;
            };
            let csv_user: &str = r.get(0).unwrap();
            let csv_pass: &str = r.get(1).unwrap();

            if csv_user == user && csv_pass == passowrd {
                return Some(csv_user.to_string());
            }
        }

        None
    }
    pub fn add_user(&mut self, user: &str, hashed_pw: &str) {
        let mut db: csv::Writer<BufWriter<File>> = self.open_database_write();
        db.write_record(&[
            user, hashed_pw
        ]).unwrap();
    }
    pub(crate) fn update_user(&mut self, user: &str, base64_pass: &str) {
        let all = self.get_all();
        let all = all.iter().filter(|(u, _)| u != &user);

        {
            let mut db = self.open_temp_database_write();
            for (user, pw) in all {
                db.write_record(&[user, pw]).unwrap();
            }
            db.write_record(&[user, base64_pass]).unwrap();
        }
        
        self.copy_temp_database();
    }
    fn get_all(&self) -> Vec<(String, String)> {
        let mut db = self.open_database_read();
        db.records().filter_map(|r| r.ok()).map(|r| (r.get(0).unwrap().to_string(), r.get(1).unwrap().to_string())).collect()
    }
    fn open_database_read(&self) -> csv::Reader<BufReader<File>> {
        let mut rdr = csv::ReaderBuilder::new().from_reader(BufReader::new(File::open(&self.path).unwrap()));
        rdr
    }
    fn open_database_write(&mut self) -> csv::Writer<BufWriter<File>> {
        let mut rdr = csv::WriterBuilder::new().from_writer(BufWriter::new(File::create(&self.path).unwrap()));
        rdr
    }
    fn open_temp_database_write(&mut self) -> csv::Writer<BufWriter<File>> {
        let mut rdr = csv::WriterBuilder::new().from_writer(BufWriter::new(File::create(&format!("{}.tmp", self.path)).unwrap()));
        rdr
    }
    fn copy_temp_database(&mut self) {
        fs::rename(&self.path, &format!("{}.bak", self.path)).unwrap();
        fs::rename(&format!("{}.tmp", self.path), &self.path).unwrap();
    }
}

struct SongDatabase {
    path: String,
}

impl SongDatabase {
    pub fn new(path: String) -> Self {
        SongDatabase { path }
    }

    pub fn get_song_by_title_and_artist(&self, title: &str, artist: &str) -> Option<SongEntry<'static>> {
        let mut db = self.open_database_read();
        for r in db.records() {
            let Ok(r) = r else {
                return None;
            };

            let (csv_title, csv_artist) = (r.get(0).unwrap(), r.get(1).unwrap());

            let mut artists = csv_artist.split('␟');

            if title == csv_title && artists.any(|a| a == artist) {
                let album = r.get(2).unwrap();
                let genres = r.get(3).unwrap();
                let song_path = r.get(4).unwrap();
                
                return Some(SongEntry {
                    title: Cow::Owned(csv_title.to_string()),
                    artists: csv_artist.split('␟').into_iter().map(|a| Cow::Owned(a.to_string())).collect(),
                    album: album.to_string().into(),
                    genres: genres.split('␟').into_iter().map(|a| Cow::Owned(a.to_string())).collect(),
                    song_path: song_path.to_string().into(),
                });
            }
        }

        None
    }

    pub fn get_all_json(&self) -> String {
        let mut db = self.open_database_read();
        let mut json = String::from("[");
        for r in db.records() {
            let Ok(r) = r else {
                break;
            };

            let (csv_title, csv_artist) = (r.get(0).unwrap(), r.get(1).unwrap());
            let album = r.get(2).unwrap();
            let genres = r.get(3).unwrap();
            let song_path = r.get(4).unwrap();
                
            json.push_str(&serde_json::to_string(&SongEntry {
                title: Cow::Borrowed(csv_title),
                artists: csv_artist.split('␟').into_iter().map(|a| Cow::Borrowed(a)).collect(),
                album: album.into(),
                genres: genres.split('␟').into_iter().map(|a| Cow::Borrowed(a)).collect(),
                song_path: song_path.into(),
            }).unwrap());
            json.push_str(",");
        }

        json.replace_range(json.len() - 1.., "]");
        
        json
    }

    pub fn get_all(&self) -> Vec<SongEntry<'static>> {
        let mut db = self.open_database_read();
        let mut entries = Vec::new();
        for r in db.records() {
            let Ok(r) = r else {
                break;
            };

            let (csv_title, csv_artist) = (r.get(0).unwrap(), r.get(1).unwrap());
            let album = r.get(2).unwrap();
            let genres = r.get(3).unwrap();
            let song_path = r.get(4).unwrap();
                
            entries.push(SongEntry {
                title: csv_title.to_string().into(),
                artists: csv_artist.split('\x1F').into_iter().map(|a| a.to_string().into()).collect(),
                album: album.to_string().into(),
                genres: genres.split('\x1F').into_iter().map(|a| a.to_string().into()).collect(),
                song_path: song_path.to_string().into(),
            });
        }

        entries
    }

    pub fn add_song(&mut self, song: &SongEntry) {
        let mut db = self.open_database_write();
        db.write_record(&[
            song.title.as_ref(),
            &song.artists.join("\x1F"),
            song.album.as_ref(),
            &song.genres.join("\x1F"),
            &song.song_path,
        ]).unwrap();
    }

    fn open_database_read(&self) -> csv::Reader<BufReader<File>> {
        let mut rdr = csv::ReaderBuilder::new().delimiter(b'\x1D').from_reader(BufReader::new(File::open(&self.path).unwrap()));
        rdr
    }

    fn open_database_write(&self) -> csv::Writer<BufWriter<File>> {
        let mut rdr = csv::WriterBuilder::new().delimiter(b'\x1D').from_writer(BufWriter::new(File::options().append(true).open(&self.path).unwrap()));
        rdr
    }
}