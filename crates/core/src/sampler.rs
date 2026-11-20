//! Sampler settings of a model profile: what goes into every `/completion` request. A field
//! left out keeps llama-server's own default, except `temperature` and `n_predict`, which the
//! app always sets (the values the app sent before there were profiles).
//!
//! `n_predict` is the answer length ("Antwortlänge"): the length a reply aims for
//! ([`crate::length`]), not where it is cut off.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::length;

pub const DEFAULT_TEMPERATURE: f64 = 0.8;
/// The answer length in tokens when a profile leaves it out.
pub const DEFAULT_N_PREDICT: i64 = 800;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sampler {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typical_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_last_n: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dry_multiplier: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dry_base: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dry_allowed_length: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dry_penalty_last_n: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xtc_probability: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xtc_threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_predict: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

impl Sampler {
    /// What a profile gets when it is created from the old fixed settings.
    pub fn legacy() -> Sampler {
        Sampler { temperature: Some(DEFAULT_TEMPERATURE), n_predict: Some(DEFAULT_N_PREDICT), ..Sampler::default() }
    }

    /// Every set value inside the range llama-server accepts. Err: the name of the first field
    /// that is not.
    pub fn check(&self) -> Result<(), &'static str> {
        let f = |name: &'static str, v: Option<f64>, lo: f64, hi: f64| match v {
            Some(x) if !(x.is_finite() && (lo..=hi).contains(&x)) => Err(name),
            _ => Ok(()),
        };
        let i = |name: &'static str, v: Option<i64>, lo: i64, hi: i64| match v {
            Some(x) if !(lo..=hi).contains(&x) => Err(name),
            _ => Ok(()),
        };
        f("temperature", self.temperature, 0.0, 5.0)?;
        f("top_p", self.top_p, 0.0, 1.0)?;
        i("top_k", self.top_k, 0, 1000)?;
        f("min_p", self.min_p, 0.0, 1.0)?;
        f("typical_p", self.typical_p, 0.0, 1.0)?;
        f("repeat_penalty", self.repeat_penalty, 0.0, 5.0)?;
        i("repeat_last_n", self.repeat_last_n, -1, 1 << 20)?;
        f("presence_penalty", self.presence_penalty, -2.0, 2.0)?;
        f("frequency_penalty", self.frequency_penalty, -2.0, 2.0)?;
        f("dry_multiplier", self.dry_multiplier, 0.0, 10.0)?;
        f("dry_base", self.dry_base, 1.0, 10.0)?;
        i("dry_allowed_length", self.dry_allowed_length, 0, 1000)?;
        i("dry_penalty_last_n", self.dry_penalty_last_n, -1, 1 << 20)?;
        f("xtc_probability", self.xtc_probability, 0.0, 1.0)?;
        f("xtc_threshold", self.xtc_threshold, 0.0, 1.0)?;
        i("n_predict", self.n_predict, 1, 32_768)?;
        i("seed", self.seed, -1, u32::MAX as i64)?;
        Ok(())
    }

    /// The answer length in tokens: what a reply aims for.
    pub fn reply_length(&self) -> i64 {
        self.n_predict.unwrap_or(DEFAULT_N_PREDICT).max(1)
    }

    /// The fields for the `/completion` request: every set value, plus `temperature` with its
    /// default when the profile leaves it out. `n_predict` is the hard cap ([`length::hard_cap`]):
    /// the answer length itself, or with `reasoning` (the thinking shares the tokens) the wider
    /// [`length::cap`]. The reply itself ends at a clean sentence end shortly before the length
    /// ([`length::SoftStop`]).
    pub fn request_fields(&self, reasoning: bool) -> Map<String, Value> {
        let Value::Object(mut map) = serde_json::to_value(self).unwrap_or_default() else { return Map::new() };
        map.entry("temperature").or_insert(DEFAULT_TEMPERATURE.into());
        map.insert("n_predict".into(), length::hard_cap(self.reply_length(), reasoning).into());
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_the_old_fixed_values() {
        // n_predict goes as the hard cap: the answer length of 800 tokens, or with reasoning the wider cap.
        assert_eq!(Value::Object(Sampler::default().request_fields(false)), json!({ "temperature": 0.8, "n_predict": 800 }));
        assert_eq!(Value::Object(Sampler::default().request_fields(true)), json!({ "temperature": 0.8, "n_predict": 1000 }));
        assert_eq!(Sampler::legacy().request_fields(false), Sampler::default().request_fields(false));
        assert_eq!((Sampler::default().reply_length(), Sampler::legacy().reply_length()), (800, 800));
        let s = Sampler { temperature: Some(1.1), min_p: Some(0.05), top_k: Some(0), n_predict: Some(150), ..Sampler::default() };
        assert_eq!(Value::Object(s.request_fields(false)), json!({ "temperature": 1.1, "min_p": 0.05, "top_k": 0, "n_predict": 150 }));
        assert_eq!(s.request_fields(true)["n_predict"], 225);
        assert_eq!(s.reply_length(), 150);
    }

    #[test]
    fn ranges_and_unknown_fields() {
        assert_eq!(Sampler::legacy().check(), Ok(()));
        assert_eq!(Sampler { temperature: Some(-0.1), ..Sampler::default() }.check(), Err("temperature"));
        assert_eq!(Sampler { top_p: Some(f64::NAN), ..Sampler::default() }.check(), Err("top_p"));
        assert_eq!(Sampler { n_predict: Some(0), ..Sampler::default() }.check(), Err("n_predict"));
        assert_eq!(Sampler { seed: Some(-1), dry_base: Some(1.75), ..Sampler::default() }.check(), Ok(()));
        assert!(serde_json::from_value::<Sampler>(json!({ "temprature": 1 })).is_err(), "typos are refused");
        let s: Sampler = serde_json::from_value(json!({ "top_k": 40, "repeat_penalty": 1.05 })).unwrap();
        assert_eq!((s.top_k, s.repeat_penalty, s.temperature), (Some(40), Some(1.05), None));
    }
}
