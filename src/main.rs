use std::{env, fs, thread};

use data::Database;
use log::{debug, info};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_http::{Request, Response, ResponseBox};

mod macros;
mod data;
use macros::*;

// mod category;
// mod entries;

mod song;

fn main() {
    env_logger::init();
    musicbrainz_rs_nova::config::set_user_agent("jukbx/1.0 ( tmtu@tmtu.ee )");

    info!("Starting jukbx");

    let mut db = Database::open("./songs.csv".into(), "./users.csv".into(), "./whitelist.csv".into());

    let mut args = env::args();
    let _ = args.next();
    if let Some(arg) = args.next() {
        if arg == "useradd" {
            let user = args.next().expect("Expected username");
            let pass = args.next().expect("Expected password");

            let mut hasher = Sha256::new();
            hasher.update(pass.as_bytes());
            let hashed_pass = hasher.finalize();
            let base64_pass = base64::encode(hashed_pass);

            db.add_user(&user, &base64_pass);

            log::info!("Added new user '{user}'")
        }

        return;
    }

    let server = tiny_http::Server::http("127.0.0.1:8089").unwrap();

    info!("Listening for HTTP requests...");
    for mut req in server.incoming_requests() {
        let db = db.clone();
        thread::spawn(move || {
            let response = get_response(db, &mut req);

            debug!(
                "{} {} => {}",
                req.method(),
                req.url(),
                response.status_code().0
            );
    
            let _ = req.respond(response);
        });
    }
}

fn get_response(
    db: Database,
    req: &mut Request,
) -> ResponseBox {
    let url = req.url();
    if url.ends_with("/") || url.ends_with("/index.html") {
        let content = fs::read("index.html").unwrap();
        return Response::from_data(content).with_status_code(200).boxed();
    }

    if url.ends_with("favicon.ico") {
        return Response::from_data(include_bytes!("../favicon.ico")).with_status_code(200).boxed();
    }

    if let Some((_, path)) = url.split_once("/") {
        if path.starts_with("songs/") {
            return song::get_audio_page(&db, req);
        }
        if path.starts_with("data/") {
            return song::get_audio_data(&db, req);
        }

        match path {
            "api/login" => return login(&db, req),
            "api/updatePassword" => return update_password(&db, req),
            "api/probeSong" => return song::probe(&db, req),
            "api/addSong" => return song::add(&db, req),
            "api/listSongs" => return song::list(&db, req),
            // "api/listAlbums" => return album::list(db, req),
            // "api/listArtists" => return artist::list(db, req),
            // "api/listGenres" => return genre::list(db, req),
            // "api/listCategories" => return category::list_categories(db, req),
            // "api/addCategory" => return category::add_category(db, req),
            // "api/editCategory" => return category::edit_category(db, req),
            // "api/removeCategory" => return category::remove_category(db, req),
            // "api/listData" => return entries::list_data(db, req),
            // "api/editData" => return entries::edit_data(db, req),
            // "api/removeData" => return entries::remove_data(db, req),
            _ => {}
        }
    }

    Response::from_string("Not found")
        .with_status_code(404)
        .boxed()
}

fn login(db: &Database, req: &mut Request) -> ResponseBox {
    let user = try_auth!(db, req);

    #[derive(Serialize)]
    struct LoginResponse {
        user: String,
    }

    to_json!(&LoginResponse { user })
}

#[derive(Deserialize)]
struct ChangePasswordRequest {
    new_password: String,
}

fn update_password(db: &Database, req: &mut Request) -> ResponseBox {
    let user = try_auth!(db, req);
    let r: ChangePasswordRequest = try_json!(req);

    require!(r.new_password.len() > 0);
    require!(r.new_password.len() < 1000);

    let mut hasher = Sha256::new();
    hasher.update(r.new_password.as_bytes());
    let hashed_pass = hasher.finalize();
    let base64_pass = base64::encode(hashed_pass);

    db.update_user(&user, &base64_pass);
    
    Response::from_string("{}").with_status_code(200).boxed()
}
