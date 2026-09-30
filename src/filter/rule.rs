use std::{borrow::Cow, collections::HashMap};

use serde::{Deserialize, Serialize};

use crate::db::types::SqlValue;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FilterSet {
    pub columns: HashMap<String, ColumnFilter>,
}

impl FilterSet {
    pub fn is_empty(&self) -> bool {
        self.active_count() == 0
    }

    pub fn active_count(&self) -> usize {
        self.columns
            .values()
            .flat_map(|cf| cf.rules.iter())
            .filter(|r| r.enabled)
            .count()
    }
}

/// Rules on one column; a row matches when any enabled rule matches.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ColumnFilter {
    pub rules: Vec<FilterRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterRule {
    pub condition: Condition,
    pub enabled: bool,
}

impl FilterRule {
    pub fn new(condition: Condition) -> Self {
        Self {
            condition,
            enabled: true,
        }
    }
}

/// A comparison together with the operand its operator requires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    Lt(SqlValue),
    Gt(SqlValue),
    Eq(SqlValue),
    /// Case-insensitive substring match; the text is literal, not a pattern.
    Contains(String),
    Regex(String),
}

impl Condition {
    pub fn op(&self) -> FilterOp {
        match self {
            Condition::Lt(_) => FilterOp::Lt,
            Condition::Gt(_) => FilterOp::Gt,
            Condition::Eq(_) => FilterOp::Eq,
            Condition::Contains(_) => FilterOp::Contains,
            Condition::Regex(_) => FilterOp::Regex,
        }
    }

    /// The operand as the user typed it.
    pub fn operand_text(&self) -> Cow<'_, str> {
        match self {
            Condition::Lt(value) | Condition::Gt(value) | Condition::Eq(value) => value.to_text(),
            Condition::Contains(text) | Condition::Regex(text) => Cow::Borrowed(text),
        }
    }
}

/// The operators offered by the filter editor, in menu order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Lt,
    Gt,
    Eq,
    Contains,
    Regex,
}

impl FilterOp {
    pub const ALL: [FilterOp; 5] = [
        FilterOp::Lt,
        FilterOp::Gt,
        FilterOp::Eq,
        FilterOp::Contains,
        FilterOp::Regex,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            FilterOp::Lt => "<",
            FilterOp::Gt => ">",
            FilterOp::Eq => "==",
            FilterOp::Contains => "contains",
            FilterOp::Regex => "regexp",
        }
    }
}
