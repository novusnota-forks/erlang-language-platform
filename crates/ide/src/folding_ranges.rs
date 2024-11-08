/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

use elp_ide_db::elp_base_db::FileId;
use elp_ide_db::RootDatabase;
use elp_syntax::TextRange;
use hir::form_list::DocAttribute;
use hir::form_list::ModuleDocAttribute;
use hir::FunctionDef;
use hir::InFile;
use hir::RecordDef;
use hir::Semantic;

#[derive(Debug, PartialEq, Eq)]
pub enum FoldingRangeKind {
    Function,
    Record,
    ModuleDocAttribute,
    DocAttribute,
}

#[derive(Debug)]
pub struct FoldingRange {
    pub range: TextRange,
    pub kind: FoldingRangeKind,
}

pub trait FoldingRangeTrait {
    fn folding_range(&self, db: &RootDatabase) -> Option<FoldingRange>;
}

impl FoldingRangeTrait for FunctionDef {
    fn folding_range(&self, db: &RootDatabase) -> Option<FoldingRange> {
        let range = self.range(db)?;
        let folding_range = FoldingRange {
            kind: FoldingRangeKind::Function,
            range,
        };
        Some(folding_range)
    }
}

impl FoldingRangeTrait for RecordDef {
    fn folding_range(&self, db: &RootDatabase) -> Option<FoldingRange> {
        let range = self.range(db);
        let folding_range = FoldingRange {
            kind: FoldingRangeKind::Record,
            range,
        };
        Some(folding_range)
    }
}

impl FoldingRangeTrait for InFile<&ModuleDocAttribute> {
    fn folding_range(&self, db: &RootDatabase) -> Option<FoldingRange> {
        let range = self.value.form_id.range(db, self.file_id);
        let folding_range = FoldingRange {
            kind: FoldingRangeKind::ModuleDocAttribute,
            range,
        };
        Some(folding_range)
    }
}

impl FoldingRangeTrait for InFile<&DocAttribute> {
    fn folding_range(&self, db: &RootDatabase) -> Option<FoldingRange> {
        let range = self.value.form_id.range(db, self.file_id);
        let folding_range = FoldingRange {
            kind: FoldingRangeKind::DocAttribute,
            range,
        };
        Some(folding_range)
    }
}

// Feature: Folding
//
// Defines folding regions for functions, records and doc attributes.
pub(crate) fn folding_ranges(db: &RootDatabase, file_id: FileId) -> Vec<FoldingRange> {
    let mut folds = Vec::new();
    let sema = Semantic::new(db);
    let def_map = sema.def_map(file_id);
    let form_list = sema.form_list(file_id);
    // Functions
    for (_, def) in def_map.get_functions() {
        if let Some(folding_range) = def.folding_range(db) {
            folds.push(folding_range)
        }
    }
    // Records
    for def in def_map.get_records().values() {
        if let Some(folding_range) = def.folding_range(db) {
            folds.push(folding_range)
        }
    }
    // Module Doc Attributes
    for (_idx, attribute) in form_list.module_doc_attributes() {
        let in_file = InFile::new(file_id, attribute);
        if let Some(folding_range) = in_file.folding_range(db) {
            folds.push(folding_range)
        }
    }
    // Doc Attributes
    for (_idx, attribute) in form_list.doc_attributes() {
        let in_file = InFile::new(file_id, attribute);
        if let Some(folding_range) = in_file.folding_range(db) {
            folds.push(folding_range)
        }
    }
    folds
}

#[cfg(test)]
mod tests {
    use elp_ide_db::elp_base_db::fixture::extract_tags;

    use super::*;
    use crate::fixture;

    fn check(fixture: &str) {
        let (ranges, fixture) = extract_tags(fixture.trim_start(), "fold");
        let (analysis, file_id) = fixture::single_file(&fixture);
        let mut folding_ranges = analysis.folding_ranges(file_id).unwrap_or_default();
        folding_ranges
            .sort_by_key(|folding_range| (folding_range.range.start(), folding_range.range.end()));

        assert_eq!(
            folding_ranges.len(),
            ranges.len(),
            "The amount of folds is different than the expected amount"
        );

        for (folding_range, (range, attr)) in folding_ranges.iter().zip(ranges.into_iter()) {
            assert_eq!(
                folding_range.range.start(),
                range.start(),
                "mismatched start of folding ranges"
            );
            assert_eq!(
                folding_range.range.end(),
                range.end(),
                "mismatched end of folding ranges"
            );

            let kind = match folding_range.kind {
                FoldingRangeKind::Function
                | FoldingRangeKind::Record
                | FoldingRangeKind::ModuleDocAttribute
                | FoldingRangeKind::DocAttribute => "region",
            };
            assert_eq!(kind, &attr.unwrap());
        }
    }

    #[test]
    fn test_function() {
        check(
            r#"
-module(my_module).
<fold region>one() ->
  ok.</fold>
"#,
        )
    }

    #[test]
    fn test_record() {
        check(
            r#"
-module(my_module).
<fold region>-record(my_record, {a :: integer(), b :: binary()}).</fold>
"#,
        )
    }

    #[test]
    fn test_records_and_functions() {
        check(
            r#"
-module(my_module).

<fold region>-record(my_record, {a :: integer(),
                                 b :: binary()}).</fold>

<fold region>one() ->
  ok.</fold>

<fold region>two() ->
  ok,
  ok.</fold>
"#,
        );
    }

    #[test]
    fn test_module_doc_attributes() {
        check(
            r#"
-module(my_module).
<fold region>-moduledoc """
This is a module doc
""".</fold>

-export([one/0]).

<fold region>one() -> 1.</fold>
"#,
        );
    }

    #[test]
    fn test_doc_attributes() {
        check(
            r#"
-module(my_module).

-export([one/0]).

<fold region>-doc "
This is one function
".</fold>
<fold region>one() -> 1.</fold>
"#,
        );
    }
}
