use serde::{Deserialize, Serialize};

/// Connection envelope exchanged during signaling
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionEnvelope {
    pub sdp: String,
    pub machine_id: String,
    pub hostname: String,
    pub version: u32,
}

/// Get or create the machine ID
pub fn get_machine_id() -> anyhow::Result<String> {
    let path = crate::config::machine_id_path()?;

    if path.exists() {
        return Ok(std::fs::read_to_string(&path)?.trim().to_string());
    }

    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &id)?;
    Ok(id)
}

/// Generate a QR code string for terminal display
pub fn generate_qr_terminal(data: &str) -> String {
    use qrcode::QrCode;

    match QrCode::new(data) {
        Ok(code) => {
            let string = code
                .render::<char>()
                .quiet_zone(false)
                .module_dimensions(2, 1)
                .build();
            string
        }
        Err(e) => format!("(QR generation failed: {})", e),
    }
}
