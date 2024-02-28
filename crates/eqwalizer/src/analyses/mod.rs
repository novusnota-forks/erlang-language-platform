/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

use std::sync::Arc;

use elp_base_db::ModuleName;
use elp_base_db::ProjectId;
use elp_types_db::eqwalizer::EqwalizerDiagnostic;

use crate::ast::db::EqwalizerASTDatabase;

mod escape_hatches;

#[salsa::query_group(EqwalizerAnalysesDatabaseStorage)]
pub trait EqwalizerAnalysesDatabase: EqwalizerASTDatabase {
    fn compute_eqwalizer_stats(
        &self,
        project_id: ProjectId,
        module: ModuleName,
    ) -> Arc<Vec<EqwalizerDiagnostic>>;
}

pub fn compute_eqwalizer_stats(
    db: &dyn EqwalizerAnalysesDatabase,
    project_id: ProjectId,
    module: ModuleName,
) -> Arc<Vec<EqwalizerDiagnostic>> {
    let mut diagnostics = vec![];
    escape_hatches::escape_hatches(db, &mut diagnostics, project_id, module);
    Arc::new(diagnostics)
}
