use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Handshake {
    pub password_hash_algo: String,
    pub password_hash_iterations: String,
    pub totp: String,
    pub nonce: String,
    pub compression: String,
}

#[derive(Deserialize, Debug)]
pub struct HandshakeResponse {
    pub id: String,
    pub htb: Handshake,
}

#[derive(Deserialize, Debug)]
pub struct Info {
    pub id: String,
    pub inf: (String, Option<String>),
}

#[derive(Deserialize, Debug)]
pub struct Buffer {
    pub ptr_buffer: String,
    pub number: i32,
    pub short_name: Option<String>,
    pub full_name: String,
    pub title: Option<String>,

    #[serde(skip, default = "default_hotlist")]
    pub hotlist: (i32, i32, i32, i32),
}

#[derive(Deserialize, Debug)]
pub struct BuffersResponse {
    pub id: String,
    pub hda: Vec<Buffer>,
}

#[derive(Deserialize, Debug)]
pub struct Hotlist {
    pub priority: i32,
    pub buffer: String,
    pub count: (i32, i32, i32, i32), // counts per urgency least -> most
}

#[derive(Deserialize, Debug)]
pub struct Hdata<T> {
    pub id: String,
    pub hda: Vec<T>,
}

#[derive(Deserialize, Debug)]
pub struct LineAddr {
    pub line: String,
}

#[derive(Deserialize, Debug)]
pub struct LineData {
    pub ptr_line: Option<String>,
    pub buffer: String,
    pub date: String,
    pub displayed: u8,
    pub highlight: u8,
    pub prefix: Option<String>,
    pub message: String,
    /// -1 disabled, 0 low, 1 message, 2 private, 3 highlight
    pub notify_level: i8,
}

#[derive(Deserialize, Debug)]
pub struct LineAddedEvent {
    pub id: String,
    pub hda: (LineData,),
}

fn default_hotlist() -> (i32, i32, i32, i32) {
    (0, 0, 0, 0)
}

#[derive(Deserialize, Debug)]
pub struct CompletionData {
    pub context: String,
    pub base_word: String,
    pub pos_start: i32,
    pub pos_end: i32,
    pub add_space: u8,
    pub list: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct CompletionResponse {
    pub id: String,
    pub hda: Vec<CompletionData>,
}
