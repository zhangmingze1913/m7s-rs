use m7s_rs::license::{Claims, sign};

fn main() {
    let claims = Claims {
        edition: "pro".to_string(),
        expires: 2_000_000_000,
        seats: std::env::var("M7S_LICENSE_SEATS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10),
        customer: "demo".to_string(),
    };
    match sign(&claims) {
        Ok(token) => println!("{token}"),
        Err(e) => eprintln!("{e}"),
    }
}
