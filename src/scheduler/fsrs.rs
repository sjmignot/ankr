use chrono::Utc;
use rs_fsrs::{BasicScheduler, Card as FsrsCard, ImplScheduler, Parameters, Rating as FsrsRating, State};
use crate::models::*;
use crate::db::queries::{now_unix, today_day};

pub struct FsrsScheduler {
    pub params: Parameters,
    pub desired_retention: f32,
}

impl FsrsScheduler {
    pub fn new(desired_retention: f32) -> Self {
        Self {
            params: Parameters::default(),
            desired_retention,
        }
    }

    pub fn schedule(&self, card: &Card, rating: Rating, crt: i64) -> CardState {
        let fsrs_card = self.card_to_fsrs(card);
        let now = Utc::now();
        let mut scheduler = BasicScheduler::new(self.params.clone(), fsrs_card, now);
        let fsrs_rating = to_fsrs_rating(rating);
        let info = scheduler.review(fsrs_rating);
        let next = info.card;

        let today = today_day(crt);
        let due_days = (next.due - now).num_days().max(0);
        let scheduled_days = next.scheduled_days.max(0);

        let (new_type, new_queue) = match next.state {
            State::New => (CardType::New, Queue::New),
            State::Learning => (CardType::Learning, Queue::Learning),
            State::Review => (CardType::Review, Queue::Review),
            State::Relearning => (CardType::Relearning, Queue::Learning),
        };

        let new_lapses = if rating == Rating::Again && card.card_type == CardType::Review {
            card.lapses + 1
        } else {
            card.lapses
        };

        let was_new = card.card_type == CardType::New;
        let new_reps = card.reps + 1;

        // factor: store stability * 1000 as an integer (Anki uses ease * 1000, ~2500 default)
        // We repurpose factor to carry stability for FSRS.
        let factor = (next.stability * 1000.0) as i64;

        CardState {
            stability: next.stability as f32,
            difficulty: next.difficulty as f32,
            due_days,
            interval: scheduled_days,
            new_reps,
            new_lapses,
            card_type: new_type,
            queue: new_queue,
            factor: factor.max(1300), // Anki minimum
        }
    }

    fn card_to_fsrs(&self, card: &Card) -> FsrsCard {
        // Parse FSRS state from card.data JSON: {"s": stability, "d": difficulty}
        let (stability, difficulty) = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&card.data) {
            let s = v.get("s").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let d = v.get("d").and_then(|x| x.as_f64()).unwrap_or(0.0);
            (s, d)
        } else {
            (0.0, 0.0)
        };

        let state = match card.card_type {
            CardType::New => State::New,
            CardType::Learning => State::Learning,
            CardType::Review => State::Review,
            CardType::Relearning => State::Relearning,
        };

        // Calculate last_review from current due and interval
        let now = Utc::now();
        let last_review = if card.ivl > 0 {
            now - chrono::Duration::days(card.ivl)
        } else {
            now
        };

        FsrsCard {
            stability,
            difficulty,
            state,
            reps: card.reps,
            lapses: card.lapses,
            elapsed_days: card.ivl.max(0),
            scheduled_days: card.ivl.max(0),
            due: now,
            last_review,
        }
    }
}

fn to_fsrs_rating(r: Rating) -> FsrsRating {
    match r {
        Rating::Again => FsrsRating::Again,
        Rating::Hard => FsrsRating::Hard,
        Rating::Good => FsrsRating::Good,
        Rating::Easy => FsrsRating::Easy,
    }
}
