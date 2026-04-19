//! SWC-backed minifier for mJS scripts.
//!
//! Output is targeted at ES5 because the on-device Mongoose JS engine
//! doesn't support arrow functions, template literals, classes,
//! destructuring, etc. — features the minifier would otherwise emit
//! when collapsing expressions.

use std::sync::Arc;

use swc_common::{sync::Lrc, FileName, Globals, Mark, SourceMap, Spanned, GLOBALS};
use swc_ecma_ast::{EsVersion, Program};
use swc_ecma_codegen::{
    text_writer::{omit_trailing_semi, JsWriter},
    Config as CodegenConfig, Emitter,
};
use swc_ecma_minifier::{
    optimize,
    option::{CompressOptions, ExtraOptions, MangleOptions, MinifyOptions, SimpleMangleCache},
};
use swc_ecma_parser::{error::Error as ParseError, parse_file_as_script, Syntax};
use swc_ecma_transforms_base::{
    fixer::{fixer, paren_remover},
    resolver,
};

fn format_parse_error(cm: &SourceMap, e: &ParseError) -> String {
    let loc = cm.lookup_char_pos(e.span().lo);
    format!(
        "{}:{}:{}: {}",
        loc.file.name,
        loc.line,
        loc.col.0 + 1,
        e.kind().msg()
    )
}

#[derive(Debug)]
pub struct MinifyError(String);

impl std::fmt::Display for MinifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MinifyError {}

/// Minify a JS source string. `source_name` is only used for diagnostic
/// labels (parse/codegen errors reference it), so pass the real file
/// path where available.
pub fn minify(source: &str, source_name: &str) -> Result<String, MinifyError> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        Lrc::new(FileName::Custom(source_name.into())),
        source.to_string(),
    );

    let globals = Globals::new();
    GLOBALS.set(&globals, || {
        let mut errors = Vec::new();
        let script = parse_file_as_script(
            &fm,
            Syntax::Es(Default::default()),
            EsVersion::Es5,
            None,
            &mut errors,
        )
        .map_err(|e| MinifyError(format!("parse error: {}", format_parse_error(&cm, &e))))?;

        if !errors.is_empty() {
            let joined = errors
                .iter()
                .map(|e| format_parse_error(&cm, e))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(MinifyError(format!("parse errors: {joined}")));
        }

        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();

        let program = Program::Script(script)
            .apply(resolver(unresolved_mark, top_level_mark, false))
            .apply(paren_remover(None));

        let program = optimize(
            program,
            cm.clone(),
            None,
            None,
            &MinifyOptions {
                compress: Some(CompressOptions {
                    arrows: false,
                    ecma: EsVersion::Es5,
                    ..Default::default()
                }),
                mangle: Some(MangleOptions {
                    top_level: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            },
            &ExtraOptions {
                unresolved_mark,
                top_level_mark,
                mangle_name_cache: Some(Arc::new(SimpleMangleCache::default())),
            },
        );

        let program = program.apply(fixer(None));

        let mut buf = Vec::new();
        {
            let mut emitter = Emitter {
                cfg: CodegenConfig::default()
                    .with_minify(true)
                    .with_target(EsVersion::Es5),
                cm: cm.clone(),
                comments: None,
                wr: omit_trailing_semi(JsWriter::new(cm.clone(), "\n", &mut buf, None)),
            };
            emitter
                .emit_program(&program)
                .map_err(|e| MinifyError(format!("codegen error: {e}")))?;
        }

        String::from_utf8(buf).map_err(|e| MinifyError(format!("non-UTF8 codegen output: {e}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn min(src: &str) -> String {
        minify(src, "test.js").expect("minify should succeed")
    }

    #[test]
    fn strips_comments() {
        let out = min("// header\nlet x = 1; /* mid */ let y = 2;\n");
        assert!(!out.contains("//"));
        assert!(!out.contains("/*"));
    }

    #[test]
    fn mangles_top_level_identifier() {
        let out = min("let WASHER_SWITCH_ID = 3; print(WASHER_SWITCH_ID);");
        assert!(
            !out.contains("WASHER_SWITCH_ID"),
            "expected identifier mangling, got: {out}"
        );
    }

    #[test]
    fn preserves_unresolved_globals() {
        let out = min("Shelly.call(\"Switch.GetStatus\", { id: 0 });");
        assert!(
            out.contains("Shelly"),
            "expected Shelly to survive, got: {out}"
        );
    }

    #[test]
    fn does_not_emit_arrow_functions() {
        let out = min(
            "function tick() { Timer.set(2000, true, function () { print(\"hi\"); }); } tick();",
        );
        assert!(
            !out.contains("=>"),
            "expected no arrow functions in ES5 output, got: {out}"
        );
    }

    #[test]
    fn preserves_string_contents() {
        let out = min(r#"print("hello // not a comment /* nope */");"#);
        assert!(out.contains("hello // not a comment /* nope */"));
    }

    #[test]
    fn returns_error_on_invalid_syntax() {
        assert!(minify("function (", "test.js").is_err());
    }

    #[test]
    fn empty_input() {
        let out = min("");
        assert_eq!(out, "");
    }
}
