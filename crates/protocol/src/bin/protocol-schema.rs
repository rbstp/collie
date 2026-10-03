use std::path::Path;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: protocol-schema <dir>");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir).expect("create schema dir");
    for (name, schema) in [
        ("client-frame.schema.json", protocol::client_frame_schema()),
        ("server-frame.schema.json", protocol::server_frame_schema()),
    ] {
        let json = serde_json::to_string_pretty(&schema).expect("schema serializes") + "\n";
        std::fs::write(dir.join(name), json).expect("write schema");
    }
}
