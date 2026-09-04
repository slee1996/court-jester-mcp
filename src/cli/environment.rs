//! Scoped verification environment overrides.

use court_jester::types::NativeFuzzEngine;
use std::env;

/// Apply the CLI verification timeout to every verification stage without
/// expanding the public `VerifyOptions` literal compatibility surface.
pub(super) struct VerifyTimeoutEnv {
    previous: Vec<(String, Option<std::ffi::OsString>)>,
}

impl VerifyTimeoutEnv {
    pub(super) fn install(timeout_seconds: Option<f64>) -> Self {
        let mut previous = Vec::new();
        if let Some(timeout) = timeout_seconds {
            for key in [
                "COURT_JESTER_VERIFY_PYTHON_TIMEOUT_SECONDS",
                "COURT_JESTER_VERIFY_TYPESCRIPT_TIMEOUT_SECONDS",
                "COURT_JESTER_VERIFY_TEST_TIMEOUT_SECONDS",
            ] {
                previous.push((key.to_string(), env::var_os(key)));
                env::set_var(key, timeout.to_string());
            }
        }
        Self { previous }
    }
}

impl Drop for VerifyTimeoutEnv {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            if let Some(value) = value {
                env::set_var(key, value);
            } else {
                env::remove_var(key);
            }
        }
    }
}

pub(super) struct VerifyNativeFuzzEnv {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl VerifyNativeFuzzEnv {
    pub(super) fn install(engine: NativeFuzzEngine, runs: Option<usize>) -> Self {
        let mut previous = Vec::new();
        if engine != NativeFuzzEngine::Off {
            previous.push((
                "COURT_JESTER_NATIVE_FUZZ_ENGINE",
                env::var_os("COURT_JESTER_NATIVE_FUZZ_ENGINE"),
            ));
            env::set_var(
                "COURT_JESTER_NATIVE_FUZZ_ENGINE",
                match engine {
                    NativeFuzzEngine::Off => "off",
                    NativeFuzzEngine::Auto => "auto",
                    NativeFuzzEngine::Atheris => "atheris",
                    NativeFuzzEngine::Jazzer => "jazzer",
                },
            );
        }
        if let Some(runs) = runs {
            previous.push((
                "COURT_JESTER_NATIVE_FUZZ_RUNS",
                env::var_os("COURT_JESTER_NATIVE_FUZZ_RUNS"),
            ));
            env::set_var("COURT_JESTER_NATIVE_FUZZ_RUNS", runs.to_string());
        }
        Self { previous }
    }
}

impl Drop for VerifyNativeFuzzEnv {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            if let Some(value) = value {
                env::set_var(key, value);
            } else {
                env::remove_var(key);
            }
        }
    }
}

pub(super) struct VerifyLlmPlateauEnv {
    previous: Option<Option<std::ffi::OsString>>,
}

impl VerifyLlmPlateauEnv {
    pub(super) fn install(command: Option<&str>) -> Self {
        let previous = command.map(|command| {
            let previous = env::var_os("COURT_JESTER_LLM_PLATEAU_COMMAND");
            env::set_var("COURT_JESTER_LLM_PLATEAU_COMMAND", command);
            previous
        });
        Self { previous }
    }
}

impl Drop for VerifyLlmPlateauEnv {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(Some(previous)) => env::set_var("COURT_JESTER_LLM_PLATEAU_COMMAND", previous),
            Some(None) => env::remove_var("COURT_JESTER_LLM_PLATEAU_COMMAND"),
            None => {}
        }
    }
}
