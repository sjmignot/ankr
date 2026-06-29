use chrono::Utc;
use rs_fsrs::{BasicScheduler, Card as FsrsCard, ImplScheduler, Parameters, Rating as FsrsRating, State};
use crate::models::*;
use crate::db::queries::today_day;

pub struct FsrsScheduler {
    pub params: Parameters,
}

impl FsrsScheduler {
    pub fn new(_desired_retention: f32) -> Self {
        Self {
            params: Parameters::default(),
        }
    }

    pub fn schedule(&self, card: &Card, rating: Rating, crt: i64) -> CardState {
        let fsrs_card = self.card_to_fsrs(card);
        let now = Utc::now();
        let mut scheduler = BasicScheduler::new(self.params.clone(), fsrs_card, now);
        let fsrs_rating = to_fsrs_rating(rating);
        let info = scheduler.review(fsrs_rating);
        let next = info.card;

        let _today = today_day(crt);
        let due_ts = next.due.timestamp();
        let scheduled_days = next.scheduled_days.max(0);

        let (new_type, new_queue) = match next.state {
            State::New => (CardType::New, Queue::New),
            State::Learning => (CardType::Learning, Queue::Learning),
            State::Review => (CardType::Review, Queue::Review),
            State::Relearning => (CardType::Relearning, Queue::Learning),
        };
        // Review cards must always move at least one day forward; FSRS can
        // schedule < 1-day intervals for "Hard" on short-interval cards.
        let due_days = if new_type == CardType::Review {
            (next.due - now).num_days().max(1)
        } else {
            (next.due - now).num_days().max(0)
        };

        let new_lapses = if rating == Rating::Again && card.card_type == CardType::Review {
            card.lapses + 1
        } else {
            card.lapses
        };

        let new_reps = card.reps + 1;

        // Preserve the card's existing ease factor. Anki with FSRS keeps a
        // traditional ease factor (1300–9999) in this column; FSRS state
        // (stability, difficulty) lives only in cards.data. Writing stability*1000
        // here causes the sync server to reject the chunk with HTTP 400.
        let factor = if card.factor >= 1300 && card.factor <= 9999 {
            card.factor
        } else {
            2500 // reset any previously-corrupted value
        };

        CardState {
            stability: next.stability as f32,
            difficulty: next.difficulty as f32,
            due_days,
            due_ts,
            interval: scheduled_days,
            new_reps,
            new_lapses,
            card_type: new_type,
            queue: new_queue,
            factor,
        }
    }

    fn card_to_fsrs(&self, card: &Card) -> FsrsCard {
        // Parse FSRS state from card.data JSON: {"s": stability, "d": difficulty}
        let (mut stability, mut difficulty) = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&card.data) {
            let s = v.get("s").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let d = v.get("d").and_then(|x| x.as_f64()).unwrap_or(0.0);
            (s, d)
        } else {
            (0.0, 0.0)
        };

        // Cards reviewed in Anki (not ankr) have no s/d stored, or store them
        // in a format we can't parse. For review cards, estimate stability from
        // the current interval: at the due date retrievability ≈ desired retention,
        // so stability ≈ ivl is a safe bootstrap value. Without this, feeding
        // stability=0 to the FSRS review formula produces NaN (0^{-w} = ∞ · 0).
        if stability <= 0.0 && card.card_type == CardType::Review && card.ivl > 0 {
            stability = card.ivl as f64;
        }
        // FSRS default difficulty is ~5; 0 produces nonsensical difficulty updates.
        if difficulty <= 0.0 {
            difficulty = 5.0;
        }

        let state = match card.card_type {
            CardType::New => State::New,
            CardType::Learning => State::Learning,
            CardType::Review => State::Review,
            CardType::Relearning => State::Relearning,
        };

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
