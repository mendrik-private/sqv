#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub tables: Vec<TableMeta>,
    pub views: Vec<String>,
    pub indexes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableMeta {
    pub name: String,
    pub columns: Vec<Column>,
    pub foreign_keys: Vec<ForeignKey>,
    pub row_identity: Option<RowIdentity>,
}

impl Schema {
    pub fn table(&self, name: &str) -> Option<&TableMeta> {
        self.tables.iter().find(|table| table.name == name)
    }
}

impl TableMeta {
    pub fn foreign_key(&self, column: &str) -> Option<&ForeignKey> {
        self.foreign_keys.iter().find(|fk| fk.from_col == column)
    }

    /// Per column, whether it references another table.
    pub fn foreign_key_flags(&self) -> Vec<bool> {
        self.columns
            .iter()
            .map(|column| self.foreign_key(&column.name).is_some())
            .collect()
    }

    /// Only rowid tables can undo inserts and deletes, which need a stable
    /// identity to target the restored row.
    pub fn has_mutable_rowid(&self) -> bool {
        matches!(self.row_identity, Some(RowIdentity::RowidAlias(_)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowIdentity {
    RowidAlias(String),
    PrimaryKey(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub col_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    pub is_pk: bool,
    pub pk_position: i64,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForeignKey {
    pub from_col: String,
    pub to_table: String,
    pub to_col: String,
}
