use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    New = 0,
    Learning = 1,
    Review = 2,
    Relearning = 3,
}

impl TryFrom<i32> for CardType {
    type Error = ();
    fn try_from(v: i32) -> std::result::Result<Self, ()> {
        match v {
            0 => Ok(Self::New),
            1 => Ok(Self::Learning),
            2 => Ok(Self::Review),
            3 => Ok(Self::Relearning),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Suspended = -2,
    Buried = -1,
    New = 0,
    Learning = 1,
    Review = 2,
    DayLearning = 3,
}

impl TryFrom<i32> for Queue {
    type Error = ();
    fn try_from(v: i32) -> std::result::Result<Self, ()> {
        match v {
            -2 => Ok(Self::Suspended),
            -1 => Ok(Self::Buried),
            0 => Ok(Self::New),
            1 => Ok(Self::Learning),
            2 => Ok(Self::Review),
            3 => Ok(Self::DayLearning),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}


#[derive(Debug, Clone)]
pub struct Card {
    pub id: i64,
    pub nid: i64,
    pub ord: i32,
    pub card_type: CardType,
    pub ivl: i64,
    pub factor: i64,
    pub reps: i32,
    pub lapses: i32,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub id: i64,
    pub mid: i64,
    pub flds: String,
    pub tags: String,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Template {
    pub ord: i32,
    pub qfmt: String,
    pub afmt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Standard = 0,
    Cloze = 1,
}

#[derive(Debug, Clone)]
pub struct NoteType {
    pub name: String,
    pub kind: NoteKind,
    pub fields: Vec<FieldDef>,
    pub templates: Vec<Template>,
}

#[derive(Debug, Clone)]
pub struct ResolvedNote {
    pub id: i64,
    pub notetype: NoteType,
    pub fields: Vec<(String, String)>,
    pub tags: Vec<String>,
}

impl ResolvedNote {
    pub fn first_field(&self) -> &str {
        self.fields.first().map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct Deck {
    pub id: i64,
    pub name: String,
}


#[derive(Debug, Clone)]
pub struct CardState {
    pub stability: f32,
    pub difficulty: f32,
    pub due_days: i64,
    pub due_ts: i64,
    pub interval: i64,
    pub new_reps: i32,
    pub new_lapses: i32,
    pub card_type: CardType,
    pub queue: Queue,
    pub factor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub card_id: i64,
    pub note_id: i64,
    pub rating: u8,
    pub time_taken_ms: u32,
    pub old_ivl: i64,
    pub old_factor: i64,
    pub old_type: i32,
    pub new_state: SerializableCardState,
    pub reviewed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCardState {
    pub stability: f32,
    pub difficulty: f32,
    pub due_days: i64,
    pub due_ts: i64,
    pub interval: i64,
    pub new_reps: i32,
    pub new_lapses: i32,
    pub card_type: i32,
    pub queue: i32,
    pub factor: i64,
}

impl From<CardState> for SerializableCardState {
    fn from(s: CardState) -> Self {
        Self {
            stability: s.stability,
            difficulty: s.difficulty,
            due_days: s.due_days,
            due_ts: s.due_ts,
            interval: s.interval,
            new_reps: s.new_reps,
            new_lapses: s.new_lapses,
            card_type: s.card_type as i32,
            queue: s.queue as i32,
            factor: s.factor,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewCard {
    pub text: String,
    pub back: String,
    pub tags: Vec<String>,
    pub deck_id: i64,
    pub notetype_id: i64,
}

#[derive(Debug, Default, Clone)]
pub struct SessionStats {
    pub reviewed: u32,
    pub again: u32,
    pub hard: u32,
    pub good: u32,
    pub easy: u32,
    pub new_introduced: u32,
}
