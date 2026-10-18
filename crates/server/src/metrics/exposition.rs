// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The Prometheus text exposition format (version 0.0.4), written by hand.
//!
//! Everything this server reports is a gauge with an integer value, so that is
//! all the renderer supports. A sample whose value is unknown is never built in
//! the first place, and a family left without samples is not written at all:
//! reporting an unknown as `0` would look like a real reading and keep an
//! `absent()` alert from firing.

/// The `Content-Type` a scraper expects for this format.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// One label on a sample. Only built through [`Sample::label`], which drops a
/// label whose value is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Label {
    name: &'static str,
    value: String,
}

/// A single reading of a metric, with its labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    labels: Vec<Label>,
    value: i64,
}

impl Sample {
    /// A sample with no labels yet.
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self {
            labels: Vec::new(),
            value,
        }
    }

    /// Adds a label. `name` is one of this module's callers' constants, never
    /// user input; `value` is escaped when the sample is written.
    #[must_use]
    pub fn label(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.labels.push(Label {
            name,
            value: value.into(),
        });
        self
    }

    /// Adds a label only when its value is known. An absent label and an empty
    /// one mean the same thing to Prometheus, so leaving it out is not a loss.
    #[must_use]
    pub fn label_opt(self, name: &'static str, value: Option<impl Into<String>>) -> Self {
        match value {
            Some(value) => self.label(name, value),
            None => self,
        }
    }
}

/// A metric name, its help text, and every sample reported for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    name: &'static str,
    help: &'static str,
    samples: Vec<Sample>,
}

impl Family {
    /// A gauge family with no samples yet.
    #[must_use]
    pub const fn gauge(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            samples: Vec::new(),
        }
    }

    /// Adds a sample.
    pub fn push(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    /// Adds a sample only when its value is known.
    pub fn push_opt(&mut self, value: Option<i64>, labels: impl FnOnce(Sample) -> Sample) {
        if let Some(value) = value {
            self.push(labels(Sample::new(value)));
        }
    }

    /// The samples reported so far.
    #[must_use]
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }
}

/// Escapes a label value: backslash, double quote and line feed.
fn escape_label_value(value: &str, out: &mut String) {
    value.chars().for_each(|c| match c {
        '\\' => out.push_str("\\\\"),
        '"' => out.push_str("\\\""),
        '\n' => out.push_str("\\n"),
        other => out.push(other),
    });
}

/// Escapes help text: backslash and line feed. Quotes are legal there.
fn escape_help(help: &str, out: &mut String) {
    help.chars().for_each(|c| match c {
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        other => out.push(other),
    });
}

fn write_sample(name: &str, sample: &Sample, out: &mut String) {
    out.push_str(name);
    if !sample.labels.is_empty() {
        out.push('{');
        sample.labels.iter().enumerate().for_each(|(index, label)| {
            if index > 0 {
                out.push(',');
            }
            out.push_str(label.name);
            out.push_str("=\"");
            escape_label_value(&label.value, out);
            out.push('"');
        });
        out.push('}');
    }
    out.push(' ');
    out.push_str(&sample.value.to_string());
    out.push('\n');
}

fn write_family(family: &Family, out: &mut String) {
    out.push_str("# HELP ");
    out.push_str(family.name);
    out.push(' ');
    escape_help(family.help, out);
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(family.name);
    out.push_str(" gauge\n");
    family
        .samples
        .iter()
        .for_each(|sample| write_sample(family.name, sample, out));
}

/// Renders `families` in the text exposition format, leaving out every family
/// that has no samples.
#[must_use]
pub fn render(families: &[Family]) -> String {
    let mut out = String::new();
    families
        .iter()
        .filter(|family| !family.samples.is_empty())
        .for_each(|family| write_family(family, &mut out));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_help_type_and_samples() {
        let mut family = Family::gauge("assimilate_test", "A test gauge.");
        family.push(Sample::new(3).label("host", "web-01"));
        family.push(Sample::new(-1));

        assert_eq!(
            render(&[family]),
            "# HELP assimilate_test A test gauge.\n# TYPE assimilate_test \
             gauge\nassimilate_test{host=\"web-01\"} 3\nassimilate_test -1\n"
        );
    }

    #[test]
    fn joins_several_labels_with_commas_in_insertion_order() {
        let mut family = Family::gauge("m", "h");
        family.push(Sample::new(1).label("b", "2").label("a", "1"));

        assert!(render(&[family]).contains("m{b=\"2\",a=\"1\"} 1\n"));
    }

    #[test]
    fn escapes_backslash_quote_and_newline_in_label_values() {
        let mut family = Family::gauge("m", "h");
        family.push(Sample::new(1).label("schedule", "a\\b \"c\"\nd"));

        assert!(render(&[family]).contains("m{schedule=\"a\\\\b \\\"c\\\"\\nd\"} 1\n"));
    }

    #[test]
    fn escapes_backslash_and_newline_but_not_quotes_in_help() {
        let family = {
            let mut family = Family::gauge("m", "say \"hi\"\\\nbye");
            family.push(Sample::new(1));
            family
        };

        assert!(render(&[family]).starts_with("# HELP m say \"hi\"\\\\\\nbye\n"));
    }

    #[test]
    fn leaves_unicode_label_values_alone() {
        let mut family = Family::gauge("m", "h");
        family.push(Sample::new(1).label("host", "b\u{fc}ro-\u{1f5a5}"));

        assert!(render(&[family]).contains("m{host=\"b\u{fc}ro-\u{1f5a5}\"} 1\n"));
    }

    #[test]
    fn omits_a_family_without_samples() {
        let empty = Family::gauge("assimilate_empty", "Nothing known.");
        let mut known = Family::gauge("assimilate_known", "Known.");
        known.push(Sample::new(0));

        let text = render(&[empty, known]);

        assert!(!text.contains("assimilate_empty"));
        assert!(text.contains("assimilate_known 0\n"));
    }

    #[test]
    fn push_opt_skips_an_unknown_value_instead_of_reporting_zero() {
        let mut family = Family::gauge("m", "h");
        family.push_opt(None, |s| s.label("repository", "r"));
        family.push_opt(Some(0), |s| s.label("repository", "known"));

        assert_eq!(family.samples().len(), 1);
        let text = render(&[family]);
        assert!(!text.contains("repository=\"r\""));
        assert!(text.contains("m{repository=\"known\"} 0\n"));
    }

    #[test]
    fn label_opt_leaves_out_an_unknown_label() {
        let mut family = Family::gauge("m", "h");
        family.push(
            Sample::new(1)
                .label("host", "h1")
                .label_opt("borg_version", None::<String>),
        );

        assert!(render(&[family]).contains("m{host=\"h1\"} 1\n"));
    }

    #[test]
    fn renders_nothing_for_no_families() {
        assert_eq!(render(&[]), "");
    }
}
