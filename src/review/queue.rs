use std::collections::VecDeque;
use crate::models::*;

pub struct ReviewQueue {
    learning: VecDeque<Card>,
    due: VecDeque<Card>,
    new: VecDeque<Card>,
    pub stats: SessionStats,
    pub new_limit: u32,
}

impl ReviewQueue {
    pub fn new(
        learning: Vec<Card>,
        due: Vec<Card>,
        new: Vec<Card>,
        new_limit: u32,
    ) -> Self {
        let capped_new: VecDeque<Card> = new.into_iter().take(new_limit as usize).collect();
        Self {
            learning: learning.into(),
            due: due.into(),
            new: capped_new,
            stats: SessionStats::default(),
            new_limit,
        }
    }

    pub fn total_remaining(&self) -> usize {
        self.learning.len() + self.due.len() + self.new.len()
    }

    pub fn next(&mut self) -> Option<Card> {
        self.learning.pop_front()
            .or_else(|| self.due.pop_front())
            .or_else(|| self.new.pop_front())
    }

    /// Re-queue a card rated Again (goes back to learning, end of queue).
    pub fn requeue(&mut self, card: Card) {
        self.learning.push_back(card);
    }

    pub fn record(&mut self, rating: Rating, was_new: bool) {
        self.stats.reviewed += 1;
        if was_new { self.stats.new_introduced += 1; }
        match rating {
            Rating::Again => self.stats.again += 1,
            Rating::Hard  => self.stats.hard += 1,
            Rating::Good  => self.stats.good += 1,
            Rating::Easy  => self.stats.easy += 1,
        }
    }
}
