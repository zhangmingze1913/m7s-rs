use std::path::Path;

fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all("config")?;
    m7s_rs::tls::write_self_signed(Path::new("config/cert.pem"), Path::new("config/key.pem"))?;
    println!("wrote config/cert.pem and config/key.pem");
    Ok(())
}
