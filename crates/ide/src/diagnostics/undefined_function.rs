/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

// Diagnostic: undefined-function
//
// Return a warning when invoking a function which has no known definition.
// This functionality is similar to the one provided by the XRef tool which comes with OTP,
// but relies on the internal ELP database.
// Only fully qualified calls are reported by this diagnostic (e.g. `foo:bar/2`), since
// calls to undefined local functions are already reported by the Erlang linter itself (L1227).

use elp_ide_db::elp_base_db::FileId;
use hir::FunctionDef;
use hir::Semantic;
use text_edit::TextRange;

use super::Diagnostic;
use super::DiagnosticCode;
use super::Severity;
use crate::codemod_helpers::find_call_in_function;
use crate::codemod_helpers::CheckCallCtx;
use crate::FunctionMatch;

pub(crate) fn undefined_function(
    diagnostics: &mut Vec<Diagnostic>,
    sema: &Semantic,
    file_id: FileId,
) {
    sema.def_map(file_id)
        .get_functions()
        .iter()
        .for_each(|(_arity, def)| {
            if def.file.file_id == file_id {
                check_function(diagnostics, sema, def)
            }
        });
}

pub(crate) fn check_function(diags: &mut Vec<Diagnostic>, sema: &Semantic, def: &FunctionDef) {
    let matcher = FunctionMatch::any();
    find_call_in_function(
        diags,
        sema,
        def,
        &[(&matcher, ())],
        &move |CheckCallCtx {
                   target,
                   args,
                   def_fb,
                   ..
               }: CheckCallCtx<'_, ()>| {
            let arity = args.len() as u32;
            match target {
                hir::CallTarget::Remote { .. } => {
                    match target.resolve_call(arity, sema, def_fb.file_id(), &def_fb.body()) {
                        Some(_) => None,
                        None => target
                            .label(arity, sema, &def_fb.body())
                            .map(|label| (label.to_string(), "".to_string())),
                    }
                }
                // Diagnostic L1227 already covers the case for local calls, so avoid double-reporting
                hir::CallTarget::Local { .. } => None,
            }
        },
        move |sema, mut _def_fb, _target, _call_id, diag_extra, _fix_extra, range| {
            let diag = make_diagnostic(sema, def.file.file_id, range, diag_extra);
            Some(diag)
        },
    );
}

fn make_diagnostic(
    sema: &Semantic,
    file_id: FileId,
    range: TextRange,
    function_name: &str,
) -> Diagnostic {
    let message = format!("Function '{}' is undefined.", function_name);
    Diagnostic::new(DiagnosticCode::UndefinedFunction, message, range)
        .with_severity(Severity::Warning)
        .with_ignore_fix(sema, file_id)
        .experimental()
}

#[cfg(test)]
mod tests {

    use crate::tests::check_diagnostics;
    use crate::tests::check_fix;

    #[test]
    fn test_local() {
        check_diagnostics(
            r#"
  -module(main).
  main() ->
    exists(),
    not_exists().

  exists() -> ok.
            "#,
        )
    }

    #[test]
    fn test_remote() {
        check_diagnostics(
            r#"
//- /src/main.erl
  -module(main).
  main() ->
    dependency:exists(),
    dependency:not_exists().
%%  ^^^^^^^^^^^^^^^^^^^^^^^ 💡 warning: Function 'dependency:not_exists/0' is undefined.
  exists() -> ok.
//- /src/dependency.erl
  -module(dependency).
  -compile(export_all).
  exists() -> ok.
            "#,
        )
    }

    #[test]
    fn test_in_macro() {
        check_diagnostics(
            r#"
//- /src/main.erl
  -module(main).
  -define(MY_MACRO, fun() -> dep:exists(), dep:not_exists() end).
  main() ->
    ?MY_MACRO().
%%  ^^^^^^^^^^^ 💡 warning: Function 'dep:not_exists/0' is undefined.
  exists() -> ok.
//- /src/dep.erl
  -module(dep).
  -compile(export_all).
  exists() -> ok.
            "#,
        )
    }

    #[test]
    fn test_ignore_fix() {
        check_fix(
            r#"
//- /src/main.erl
-module(main).

main() ->
  dep:exists(),
  dep:not_ex~ists().

exists() -> ok.
//- /src/dep.erl
-module(dep).
-compile(export_all).
exists() -> ok.
"#,
            r#"
-module(main).

main() ->
  dep:exists(),
  % elp:ignore W0017 (undefined_function)
  dep:not_exists().

exists() -> ok.
"#,
        )
    }
}
