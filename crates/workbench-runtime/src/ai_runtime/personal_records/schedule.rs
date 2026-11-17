use chrono::{DateTime, Duration, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Repeat {
    Once,
    Daily,
    Weekly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ScheduleInput {
    pub local_time: String,
    pub time_zone: String,
    pub repeat: Repeat,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Schedule {
    pub input: ScheduleInput,
    pub generation: u64,
    pub next_at: Option<i64>,
    pub last_at: Option<i64>,
    pub completed: bool,
    pub snoozed_at: Option<i64>,
}

impl ScheduleInput {
    fn local(&self) -> Result<NaiveDateTime, String> {
        NaiveDateTime::parse_from_str(&self.local_time, "%Y-%m-%dT%H:%M")
            .map_err(|_| "pa_reminder_time_invalid".into())
    }

    fn zone(&self) -> Result<Tz, String> {
        self.time_zone
            .parse()
            .map_err(|_| "pa_time_zone_invalid".into())
    }
}

impl Schedule {
    pub fn new(input: ScheduleInput, generation: u64, now: i64) -> Result<Self, String> {
        let at = input
            .zone()?
            .from_local_datetime(&input.local()?)
            .earliest()
            .ok_or("pa_reminder_time_invalid")?
            .timestamp_millis();
        if at <= now {
            return Err("pa_reminder_in_past".into());
        }
        Ok(Self {
            input,
            generation,
            next_at: Some(at),
            last_at: None,
            completed: false,
            snoozed_at: None,
        })
    }

    fn following(&self, at: i64) -> Result<Option<i64>, String> {
        let days = match self.input.repeat {
            Repeat::Once => return Ok(None),
            Repeat::Daily => 1,
            Repeat::Weekly => 7,
        };
        let zone = self.input.zone()?;
        let time = self.input.local()?.time();
        let mut date = DateTime::from_timestamp_millis(at)
            .ok_or("pa_reminder_time_invalid")?
            .with_timezone(&zone)
            .date_naive();
        loop {
            date = date
                .checked_add_signed(Duration::days(days))
                .ok_or("pa_reminder_time_invalid")?;
            if let Some(next) = zone.from_local_datetime(&date.and_time(time)).earliest() {
                return Ok(Some(next.timestamp_millis()));
            }
        }
    }

    pub fn complete(&mut self) -> Result<(), String> {
        if self.completed {
            return Err("pa_occurrence_completed".into());
        }
        if self.last_at.is_none() {
            let at = self.next_at.ok_or("pa_reminder_missing")?;
            self.last_at = Some(at);
            self.next_at = self.following(at)?;
        }
        self.completed = true;
        self.snoozed_at = None;
        Ok(())
    }

    pub fn snooze(&mut self, minutes: u32, now: i64) -> Result<(), String> {
        if self.completed || self.last_at.is_none() || !(1..=10_080).contains(&minutes) {
            return Err("pa_snooze_invalid".into());
        }
        let at = now
            .checked_add(i64::from(minutes) * 60_000)
            .ok_or("pa_snooze_invalid")?;
        if self.next_at.is_some_and(|next| at >= next) {
            return Err("pa_snooze_overlaps_next".into());
        }
        self.snoozed_at = Some(at);
        Ok(())
    }

    pub fn due(&mut self, now: i64) -> Result<Vec<Occurrence>, String> {
        let mut due = Vec::new();
        if let Some(at) = self.snoozed_at.filter(|at| *at <= now) {
            due.push(Occurrence {
                first_at: self.last_at.ok_or("pa_reminder_missing")?,
                dispatch_at: at,
                count: 1,
            });
            self.snoozed_at = None;
        }
        if let Some(first_at) = self.next_at.filter(|at| *at <= now) {
            let mut count = 1;
            let mut last = first_at;
            let mut next = self.following(last)?;
            while let Some(at) = next.filter(|at| *at <= now) {
                last = at;
                next = self.following(at)?;
                count += 1;
            }
            self.last_at = Some(last);
            self.next_at = next;
            self.completed = false;
            due.push(Occurrence {
                first_at,
                dispatch_at: last,
                count,
            });
        }
        Ok(due)
    }
}

pub(super) struct Occurrence {
    pub first_at: i64,
    pub dispatch_at: i64,
    pub count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn daily_wall_time_skips_dst_gap_and_coalesces_missed_occurrences() {
        let mut plan = Schedule::new(
            ScheduleInput {
                local_time: "2026-03-07T02:30".into(),
                time_zone: "America/New_York".into(),
                repeat: Repeat::Daily,
            },
            1,
            utc("2026-03-07T00:00:00Z"),
        )
        .unwrap();
        let due = plan.due(utc("2026-03-09T08:00:00Z")).unwrap();
        assert_eq!(due[0].count, 2);
        assert_eq!(due[0].first_at, utc("2026-03-07T07:30:00Z"));
        assert_eq!(due[0].dispatch_at, utc("2026-03-09T06:30:00Z"));
        assert_eq!(plan.next_at, Some(utc("2026-03-10T06:30:00Z")));
        assert!(plan.due(utc("2026-03-09T08:00:00Z")).unwrap().is_empty());
    }

    #[test]
    fn snooze_and_completion_affect_only_the_current_occurrence() {
        let now = utc("2026-11-12T02:00:00Z");
        let mut plan = Schedule::new(
            ScheduleInput {
                local_time: "2026-11-12T10:00".into(),
                time_zone: "Asia/Shanghai".into(),
                repeat: Repeat::Weekly,
            },
            1,
            now - 1,
        )
        .unwrap();
        plan.due(now).unwrap();
        let next = plan.next_at;
        plan.snooze(10, now).unwrap();
        assert_eq!(plan.next_at, next);
        plan.complete().unwrap();
        assert!(plan.due(now + 600_000).unwrap().is_empty());
        assert_eq!(plan.next_at, next);
        assert_eq!(plan.due(next.unwrap()).unwrap().len(), 1);
        assert!(!plan.completed);
    }
}
