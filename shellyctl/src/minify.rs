//! SWC-backed minifier for mJS scripts.
//!
//! Output is targeted at ES5 because the on-device Mongoose JS engine
//! doesn't support arrow functions, template literals, classes,
//! destructuring, etc. — features the minifier would otherwise emit
//! when collapsing expressions.
//!
//! After codegen, the minified output is re-parsed as ES5. SWC's
//! `optimize()` has no error channel, so this round-trip is the only
//! defence against a minifier bug shipping syntactically broken bytes
//! to a device — where the failure mode is a silent script-load error.

use std::sync::Arc;

use swc_common::{sync::Lrc, FileName, Globals, Mark, SourceMap, Spanned, GLOBALS};
use swc_ecma_ast::{EsVersion, Program};
use swc_ecma_codegen::{
    text_writer::{omit_trailing_semi, JsWriter},
    Config as CodegenConfig, Emitter,
};
use swc_ecma_compat_es2015::{arrow, block_scoping};
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

fn join_parse_errors(
    cm: &SourceMap,
    fatal: Option<&ParseError>,
    recoverable: &[ParseError],
) -> String {
    let mut msgs: Vec<String> = Vec::with_capacity(recoverable.len() + 1);
    if let Some(e) = fatal {
        msgs.push(format_parse_error(cm, e));
    }
    msgs.extend(recoverable.iter().map(|e| format_parse_error(cm, e)));
    msgs.join("; ")
}

#[derive(Debug)]
pub enum MinifyError {
    /// User-supplied source failed to parse as ES5.
    Parse(String),
    /// SWC codegen failed to write its emitter buffer.
    Codegen(String),
    /// SWC emitted bytes that are not valid UTF-8.
    NonUtf8(std::string::FromUtf8Error),
    /// Minified output failed to re-parse as ES5 — almost certainly a
    /// minifier bug, and would fail to load on the device.
    Verify(String),
}

impl std::fmt::Display for MinifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MinifyError::Parse(msg) => write!(f, "parse error: {msg}"),
            MinifyError::Codegen(msg) => write!(f, "codegen error: {msg}"),
            MinifyError::NonUtf8(e) => write!(f, "non-UTF8 codegen output: {e}"),
            MinifyError::Verify(msg) => write!(
                f,
                "minified output failed to re-parse as ES5 (likely a minifier bug, would fail to load on the device): {msg}"
            ),
        }
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
    let minified = GLOBALS.set(&globals, || -> Result<String, MinifyError> {
        let mut errors = Vec::new();
        let script = parse_file_as_script(
            &fm,
            Syntax::Es(Default::default()),
            EsVersion::Es5,
            None,
            &mut errors,
        )
        .map_err(|e| MinifyError::Parse(join_parse_errors(&cm, Some(&e), &errors)))?;

        if !errors.is_empty() {
            return Err(MinifyError::Parse(join_parse_errors(&cm, None, &errors)));
        }

        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();

        let program = Program::Script(script)
            .apply(resolver(unresolved_mark, top_level_mark, false))
            .apply(paren_remover(None))
            // Downlevel ES2015 features the device's Mongoose JS rejects.
            // SWC's minifier doesn't do this — `compress.ecma = Es5` only
            // bounds what *new* code compress emits, not what's already in
            // the AST.
            .apply(arrow(unresolved_mark))
            .apply(block_scoping(unresolved_mark));

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
                .map_err(|e| MinifyError::Codegen(format!("{e}")))?;
        }

        String::from_utf8(buf).map_err(MinifyError::NonUtf8)
    })?;

    verify_es5_output(source_name, &minified)?;
    Ok(minified)
}

fn verify_es5_output(source_name: &str, minified: &str) -> Result<(), MinifyError> {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(
        Lrc::new(FileName::Custom(format!("<minified:{source_name}>"))),
        minified.to_string(),
    );
    let mut errors = Vec::new();
    let result = parse_file_as_script(
        &fm,
        Syntax::Es(Default::default()),
        EsVersion::Es5,
        None,
        &mut errors,
    );
    match result {
        Err(e) => Err(MinifyError::Verify(join_parse_errors(
            &cm,
            Some(&e),
            &errors,
        ))),
        Ok(_) if !errors.is_empty() => {
            Err(MinifyError::Verify(join_parse_errors(&cm, None, &errors)))
        }
        Ok(_) => Ok(()),
    }
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
    fn preserves_all_device_globals() {
        let out = min(r#"
            Timer.set(1000, true, function () {
                Shelly.call("KVS.Set", { key: "k", value: JSON.stringify({a:1}) });
                Shelly.call("HTTP.GET", { url: "http://x" }, function (r) {
                    print(Math.round(r.code));
                });
            });
            "#);
        for g in ["Timer", "Shelly", "JSON", "Math", "print"] {
            assert!(
                out.contains(g),
                "expected global `{g}` preserved, got: {out}"
            );
        }
        // RPC method names are string-keyed; the minifier must never touch them.
        assert!(out.contains("KVS.Set"), "RPC method name lost: {out}");
        assert!(out.contains("HTTP.GET"), "RPC method name lost: {out}");
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
    fn downlevels_let_and_const_to_var() {
        // Mongoose JS rejects `let`/`const`; the minifier must compile them
        // away. `var` is the only ES5-legal binding form, so any leak of
        // `let `/`const ` in output bricks the device script.
        let out = min("let a = 1; const b = 2; print(a + b);");
        assert!(!out.contains("let "), "let leaked into output: {out}");
        assert!(!out.contains("const "), "const leaked into output: {out}");
    }

    #[test]
    fn preserves_string_contents() {
        let out = min(r#"print("hello // not a comment /* nope */");"#);
        assert!(out.contains("hello // not a comment /* nope */"));
    }

    #[test]
    fn returns_error_on_invalid_syntax() {
        let err = minify("function (", "weird-name.js").expect_err("invalid syntax must error");
        assert!(
            matches!(err, MinifyError::Parse(_)),
            "expected Parse, got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("weird-name.js"),
            "error should reference source name: {msg}"
        );
        assert!(
            msg.contains(":1:"),
            "error should include line number: {msg}"
        );
    }

    #[test]
    fn empty_input() {
        let out = min("");
        assert_eq!(out, "");
    }

    #[test]
    fn minifies_appliance_monitor_end_to_end() {
        // The one production consumer of the minifier. If this test breaks,
        // a real script-on-device deployment broke too.
        let src = include_str!("../../scripts/appliance-monitor.js");
        let out = minify(src, "appliance-monitor.js").expect("must minify cleanly");

        assert!(!out.is_empty(), "output unexpectedly empty");
        assert!(
            out.len() < src.len() / 2,
            "expected >50% size reduction: {} -> {}",
            src.len(),
            out.len()
        );
        // CLAUDE.md hard constraint: scripts must fit in <2KB on the device.
        assert!(
            out.len() < 2048,
            "minified output {} bytes exceeds 2KB device budget",
            out.len()
        );
        // Every device-exposed global the script uses must survive mangling.
        for g in ["Shelly", "Timer", "Math", "JSON", "print"] {
            assert!(out.contains(g), "global `{g}` missing from minified output");
        }
        // RPC method names are load-bearing strings.
        for m in ["KVS.Set", "KVS.Get", "HTTP.GET", "Switch.GetStatus"] {
            assert!(
                out.contains(m),
                "RPC method `{m}` missing from minified output"
            );
        }
    }
}
