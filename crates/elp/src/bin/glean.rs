/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

#![allow(dead_code, unused)]
use std::io::Write;
use std::mem;

use anyhow::Result;
use elp::build::load;
use elp::build::types::LoadResult;
use elp::cli::Cli;
use elp_ide::elp_ide_db::elp_base_db::FileId;
use elp_ide::elp_ide_db::elp_base_db::IncludeOtp;
use elp_ide::elp_ide_db::elp_base_db::ModuleName;
use elp_ide::elp_ide_db::elp_base_db::Vfs;
use elp_ide::elp_ide_db::elp_base_db::VfsPath;
use elp_ide::elp_ide_db::LineIndex;
use elp_ide::elp_ide_db::RootDatabase;
use elp_ide::Analysis;
use elp_ide::TextRange;
use elp_project_model::DiscoverConfig;
use elp_project_model::Project;
use elp_syntax::AstNode;
use hir::db::MinDefDatabase;
use serde::Serialize;

use crate::args::Glean;

#[derive(Serialize, Debug)]
pub(crate) struct FileFact {
    #[serde(rename = "id")]
    file_id: u32,
    #[serde(rename = "key")]
    file_path: String,
}

impl FileFact {
    fn new(file_id: FileId, file_path: String) -> Self {
        Self {
            file_id: file_id.0,
            file_path,
        }
    }
}

#[derive(Serialize, Debug)]
pub(crate) struct FileLinesFact {
    key: FileLinesFactKey,
}

impl FileLinesFact {
    fn new(file_id: FileId, lengths: Vec<u32>, ends_with_new_line: bool) -> Self {
        FileLinesFact {
            key: FileLinesFactKey {
                file_id: file_id.0,
                lengths,
                ends_with_new_line,
                unicode_or_tabs: true,
            },
        }
    }
}

#[derive(Serialize, Debug)]
struct FileLinesFactKey {
    #[serde(rename = "file")]
    file_id: u32,
    lengths: Vec<u32>,
    #[serde(rename = "endsInNewline")]
    ends_with_new_line: bool,
    #[serde(rename = "hasUnicodeOrTabs")]
    unicode_or_tabs: bool,
}

#[derive(Serialize, Debug)]
pub(crate) struct FunctionDeclarationFact {
    key: FunctionDeclarationKey,
}

impl FunctionDeclarationFact {
    fn new(file_id: FileId, fqn: MFA, span: Location) -> Self {
        Self {
            key: FunctionDeclarationKey {
                file_id: file_id.0,
                fqn,
                span,
            },
        }
    }
}

#[derive(Serialize, Debug)]
struct FunctionDeclarationKey {
    #[serde(rename = "file")]
    file_id: u32,
    fqn: MFA,
    span: Location,
}

#[derive(Serialize, Debug)]
pub(crate) struct XRefFact {
    key: XRefFactKey,
}

impl XRefFact {
    fn new(file_id: FileId, xrefs: Vec<XRefFactVal>) -> Self {
        Self {
            key: XRefFactKey {
                file_id: file_id.0,
                xrefs,
            },
        }
    }
}

#[derive(Serialize, Debug)]
struct XRefFactKey {
    #[serde(rename = "file")]
    file_id: u32,
    xrefs: Vec<XRefFactVal>,
}

#[derive(Serialize, Debug)]
struct XRefFactVal {
    source: Location,
    target: MFA,
}

impl XRefFactVal {
    fn new(source: Location, target: MFA) -> Self {
        Self { source, target }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
struct MFA {
    module: String,
    name: String,
    arity: u32,
}

impl MFA {
    fn new(module: String, name: String, arity: u32) -> Self {
        Self {
            module,
            name,
            arity,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
struct Location {
    start: u32,
    length: u32,
}

impl Location {
    fn new(start: u32, length: u32) -> Self {
        Self { start, length }
    }
}

#[derive(Serialize, Debug)]
#[serde(tag = "predicate")]
pub(crate) enum Fact {
    #[serde(rename = "src.File")]
    File { facts: Vec<FileFact> },
    #[serde(rename = "src.FileLines")]
    FileLine { facts: Vec<FileLinesFact> },
    #[serde(rename = "erlang.FunctionDeclaration")]
    FunctionDeclaration { facts: Vec<FunctionDeclarationFact> },
    #[serde(rename = "erlang.XRefsViaFqnByFile")]
    XRef { facts: Vec<XRefFact> },
}

struct IndexedFacts {
    file_facts: Vec<FileFact>,
    file_line_facts: Vec<FileLinesFact>,
    declaration_facts: Vec<FunctionDeclarationFact>,
    xref_facts: Vec<XRefFact>,
}

impl IndexedFacts {
    fn new() -> Self {
        Self {
            file_facts: vec![],
            file_line_facts: vec![],
            declaration_facts: vec![],
            xref_facts: vec![],
        }
    }
}

pub struct GleanIndexer<'a> {
    loaded: LoadResult,
    analysis: Analysis,
    cli: &'a mut dyn Cli,
    args: &'a Glean,
}

impl<'a> GleanIndexer<'a> {
    pub fn new(args: &'a Glean, cli: &'a mut dyn Cli) -> Result<Self> {
        let config = DiscoverConfig::buck();
        let loaded = load::load_project_at(
            cli,
            &args.project,
            config,
            IncludeOtp::Yes,
            elp_eqwalizer::Mode::Cli,
        )?;
        let analysis = loaded.analysis();
        let indexer = Self {
            loaded,
            analysis,
            cli,
            args,
        };
        Ok(indexer)
    }

    pub fn index(mut self) -> Result<()> {
        let facts = self.index_facts()?;
        self.write_results(facts)?;
        Ok(())
    }

    fn index_file(&self, file_id: FileId, path: &VfsPath, facts: &mut IndexedFacts) -> Result<()> {
        let proj = match self.analysis.project_id(file_id)? {
            Some(proj) => proj,
            None => return Ok(()),
        };

        if self.loaded.project_id != proj {
            return Ok(());
        }

        let line_index = self.analysis.line_index(file_id)?;

        let file_fact = match self.file_fact(file_id, path) {
            Some(file_fact) => file_fact,
            None => return Ok(()),
        };
        let line_fact = self.line_fact(file_id, &line_index);
        facts.file_facts.push(file_fact);
        facts.file_line_facts.push(line_fact);

        let module_index = self.analysis.module_index(proj)?;
        if let Some(module) = module_index.module_for_file(file_id) {
            match self.declaration_fact(file_id, module) {
                Ok(decl) => facts.declaration_facts.extend(decl),
                Err(err) => {
                    log::warn!("Error while indexing declarations for {:?}: {}", &path, err)
                }
            }
        }
        Ok(())
    }

    fn index_facts(&self) -> Result<IndexedFacts> {
        let mut ctx = IndexedFacts::new();

        if let Some(module) = &self.args.module {
            let index = self.analysis.module_index(self.loaded.project_id)?;
            let file_id = index
                .file_for_module(&ModuleName::new(module))
                .expect("No module found");
            let path = self.loaded.vfs.file_path(file_id);
            self.index_file(file_id, &path, &mut ctx)?;
        } else {
            for (file_id, path) in self.loaded.vfs.iter() {
                if let Err(err) = self.index_file(file_id, path, &mut ctx) {
                    log::warn!("Error indexing file {:?}: {}", path, err);
                }
            }
        }
        Ok(ctx)
    }

    fn write_results(&mut self, mut indexed_facts: IndexedFacts) -> Result<()> {
        let facts = vec![
            Fact::File {
                facts: mem::take(&mut indexed_facts.file_facts),
            },
            Fact::FileLine {
                facts: mem::take(&mut indexed_facts.file_line_facts),
            },
            Fact::FunctionDeclaration {
                facts: mem::take(&mut indexed_facts.declaration_facts),
            },
            Fact::XRef {
                facts: mem::take(&mut indexed_facts.xref_facts),
            },
        ];
        let content = serde_json::to_string(&facts)?;
        match &self.args.to {
            Some(to) => std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(to)?
                .write_all(&content.as_bytes()),
            None => self.cli.write_all(&content.as_bytes()),
        }?;
        Ok(())
    }

    fn file_fact(&self, file_id: FileId, path: &VfsPath) -> Option<FileFact> {
        let root = self.loaded.project.root();
        let root = root.as_path();
        let file_path = path.as_path()?;
        let file_path = file_path.strip_prefix(root)?;
        let file_path = file_path.as_ref().to_str()?.into();
        Some(FileFact::new(file_id, file_path))
    }

    fn line_fact(&self, file_id: FileId, line_index: &LineIndex) -> FileLinesFact {
        let mut line = 1;
        let mut prev_offset = 0;
        let mut lengths = vec![];
        let mut ends_with_new_line = true;
        while let Some(offset) = line_index.line_at(line) {
            let curr_offset: u32 = offset.into();
            lengths.push(curr_offset - prev_offset);
            line += 1;
            prev_offset = curr_offset;
        }
        let content = String::from_utf8_lossy(self.loaded.vfs.file_contents(file_id));
        if !content.ends_with("\n") {
            ends_with_new_line = false;
            let len = if content.len() as u32 >= prev_offset {
                content.len() as u32 - prev_offset
            } else {
                0
            };
            lengths.push(len);
        }
        FileLinesFact::new(file_id, lengths, ends_with_new_line)
    }

    fn declaration_fact(
        &self,
        file_id: FileId,
        module: &ModuleName,
    ) -> Result<Vec<FunctionDeclarationFact>> {
        let result = self
            .analysis
            .with_db(|db| Self::declarations(db, file_id, module.as_str()))?;
        Ok(result)
    }

    fn declarations(
        db: &RootDatabase,
        file_id: FileId,
        module: &str,
    ) -> Vec<FunctionDeclarationFact> {
        let def_map = db.local_def_map(file_id);
        let mut result = vec![];
        for (fun, def) in def_map.get_functions() {
            let range = def.range(db);
            if let Some(range) = range {
                let loc = range.into();
                let mfa = MFA::new(module.to_string(), fun.name().to_string(), fun.arity());
                result.push(FunctionDeclarationFact::new(file_id, mfa, loc));
            }
        }
        for (ty, def) in def_map.get_types() {
            let range = def.source(db).syntax().text_range();
            let loc = range.into();
            let mfa = MFA::new(module.to_string(), ty.name().to_string(), ty.arity());
            result.push(FunctionDeclarationFact::new(file_id, mfa, loc));
        }
        for (rec, def) in def_map.get_records() {
            let range = def.source(db).syntax().text_range();
            let loc = range.into();
            let mfa = MFA::new(module.to_string(), rec.to_string(), 99);
            result.push(FunctionDeclarationFact::new(file_id, mfa, loc));
        }
        result
    }
}

impl From<TextRange> for Location {
    fn from(range: TextRange) -> Self {
        let start: u32 = range.start().into();
        let mut length: u32 = range.len().into();
        //to allign with eqwalizer indexer
        if length > 0 {
            length -= 1
        }
        Location { start, length }
    }
}

#[cfg(test)]
mod tests {

    use std::path::PathBuf;

    use elp::cli::Fake;
    use elp_project_model::test_fixture::Fixture;
    use expect_test::expect_file;

    use super::*;

    #[test]
    fn serialization_test() {
        let mut cli = Fake::default();
        let args = Glean {
            project: PathBuf::from("."),
            module: None,
            to: None,
        };
        let file_id = FileId(10071);
        let location = Location {
            start: 0,
            length: 10,
        };
        let mfa = MFA::new(
            "smax_product_catalog".into(),
            "product_visibility_update_request_iq".into(),
            0,
        );

        let file_facts = vec![
            FileFact::new(
                file_id,
                "/local/whatsapp/server/erl/groupd_service/test/p13n/grpd_p13n_new_create_group_SUITE.erl".into(),
            )
        ];
        let file_line_facts = vec![FileLinesFact::new(file_id, vec![71, 42], true)];
        let declaration_facts = vec![FunctionDeclarationFact::new(
            file_id,
            mfa.clone(),
            location.clone(),
        )];

        let xref_facts = vec![XRefFact::new(
            file_id,
            vec![XRefFactVal::new(location, mfa)],
        )];
        let facts = IndexedFacts {
            file_facts,
            file_line_facts,
            declaration_facts,
            xref_facts,
        };

        let mut indexer = GleanIndexer::new(&args, &mut cli).expect("success");

        indexer.write_results(facts).unwrap();

        let (out, err) = cli.to_strings();
        let expected = expect_file!["../resources/test/glean/serialization_test.out"];
        expected.assert_eq(&out);
        assert_eq!(err, "")
    }

    #[test]
    fn file_fact_test() {
        let spec = r#"
        //- /glean/app_glean/src/glean_module2.erl
        -module(glean_module2).
        "#;
        let result = run_spec(spec, "glean_module2");
        let file_fact = &result.file_facts;
        assert_eq!(file_fact.len(), 1);
        assert_eq!(file_fact[0].file_path.as_str(), "src/glean_module2.erl");
    }

    #[test]
    fn line_fact_with_new_line_test() {
        let spec = r#"
        //- /glean/app_glean/src/glean_module3.erl
        -module(glean_module3).
        main() ->
            bar.

        "#;
        let result = run_spec(spec, "glean_module3");
        let line_fact = &result.file_line_facts;
        assert_eq!(line_fact.len(), 1);
        assert!(&line_fact[0].key.ends_with_new_line);
        assert_eq!(&line_fact[0].key.lengths, &vec![24, 10, 9, 1]);
    }

    #[test]
    fn line_fact_without_new_line_test() {
        let spec = r#"
        //- /glean/app_glean/src/glean_module4.erl
        -module(glean_module4).
        main() ->
            bar."#;
        let result = run_spec(spec, "glean_module4");
        let line_fact = &result.file_line_facts;
        assert_eq!(line_fact.len(), 1);
        assert_eq!(line_fact[0].key.ends_with_new_line, false);
        assert_eq!(&line_fact[0].key.lengths, &vec![24, 10, 8]);
    }

    #[test]
    fn declaration_test() {
        let module = "glean_module5";
        let spec = r#"
        //- /glean/app_glean/src/glean_module5.erl
        -module(glean_module5).
        -type tree() :: {'node', tree(), tree(), any(), any()}.
        -record(user, {name = "" :: string(),
                       notes :: tree(),
                       age :: non_neg_integer(),
                       friends=[] :: [user()],
                       bio :: string() | binary()}).
        main(A) ->
            A."#;
        let result = run_spec(spec, module);
        let decl_fact = &result.declaration_facts;
        assert_eq!(decl_fact.len(), 3);
        let main = MFA::new(module.into(), "main".into(), 1);
        let typ = MFA::new(module.into(), "tree".into(), 0);
        let rec = MFA::new(module.into(), "user".into(), 99);
        assert_eq!(&decl_fact[0].key.fqn, &main);
        assert_eq!(&decl_fact[0].key.span, &Location::new(275, 16));
        assert_eq!(&decl_fact[1].key.fqn, &typ);
        assert_eq!(&decl_fact[1].key.span, &Location::new(24, 54));
        assert_eq!(&decl_fact[2].key.fqn, &rec);
        assert_eq!(&decl_fact[2].key.span, &Location::new(80, 193));
    }

    fn run_spec(spec: &str, module: &str) -> IndexedFacts {
        let mut cli = Fake::default();
        let dir = Fixture::gen_project(spec);

        let args = Glean {
            project: dir
                .into_path()
                .to_path_buf()
                .join("glean")
                .join("app_glean"),
            module: Some(module.into()),
            to: None,
        };
        let indexer = GleanIndexer::new(&args, &mut cli).expect("success");
        indexer.index_facts().expect("should be ok")
    }
}
