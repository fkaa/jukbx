use tiny_http::Request;
#[macro_export]
macro_rules! to_json {
    ($obj:expr) => {{
        let json = crate::try_unwrap!(serde_json::to_string($obj));

        Response::from_string(json)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap(),
            )
            .with_status_code(200)
            .boxed()
    }};
}

#[macro_export]
macro_rules! try_json {
    ($req:expr) => {{
        let mut content = String::new();
        if let Err(e) = $req.as_reader().read_to_string(&mut content) {
            return Response::from_string(format!("{e:?}"))
                .with_status_code(400)
                .boxed();
        }

        let result = crate::try_unwrap!(serde_json::from_str(&content));

        result
    }};
}

#[macro_export]
macro_rules! require {
    ($val:expr) => {
        if !$val {
            return Response::from_string(stringify!($val))
                .with_status_code(400)
                .boxed();
        }
    };
}

#[macro_export]
macro_rules! try_unwrap {
    ($obj:expr) => {{
        match $obj {
            Ok(result) => result,
            Err(e) => {
                return Response::from_string(format!("{e:?}"))
                    .with_status_code(400)
                    .boxed();
            }
        }
    }};
}

pub fn get_auth(req: &Request) -> Option<(String, String)> {
    let Some(header) = req
        .headers()
        .iter()
        .find(|h| h.field == "Authorization".parse().unwrap())
    else {
        log::warn!("No auth header found");
        return None;
    };

    let Some((_, b64)) = header.value.as_str().split_once(' ') else {
        log::warn!("Failed to parse auth header '{}'", header.value);
        return None;
    };

    let Ok(bytes) = base64::decode(b64.as_bytes()) else {
        log::warn!("Failed to decode base64 '{b64}'");
        return None;
    };

    let Ok(user_password_text) = std::str::from_utf8(&bytes) else {
        log::warn!("Decoded base64 is not text");
        return None;
    };

    let Some((username, password)) = user_password_text.split_once(':') else {
        return None;
    };

    Some((username.to_string(), password.to_string()))
}

#[macro_export]
macro_rules! try_auth {
    ($db:expr, $req:expr) => {{
        use sha2::{Digest, Sha256};

        let Some((user, pass)) = crate::macros::get_auth($req) else {
            return Response::from_string("")
                .with_header(
                    tiny_http::Header::from_bytes(
                        &b"WWW-Authenticate"[..],
                        &b"Basic realm=\"my realm\""[..],
                    )
                    .unwrap(),
                )
                .with_status_code(401)
                .boxed();
        };

        let mut hasher = Sha256::new();
        hasher.update(pass.as_bytes());
        let result = hasher.finalize();
        let base64_pass = base64::encode(result);

        let user: Option<String> = $db
            .get_user(&user, &base64_pass);

        let Some(user) = user else {
            return Response::from_string("Invalid login")
                .with_header(
                    tiny_http::Header::from_bytes(
                        &b"WWW-Authenticate"[..],
                        &b"Basic realm=\"my realm\""[..],
                    )
                    .unwrap(),
                )
                .with_status_code(401)
                .boxed();
        };

        user
    }};
}
