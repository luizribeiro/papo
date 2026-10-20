{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      crane,
      nixpkgs,
      flake-utils,
      git-hooks,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        stableToolchain = pkgs.rust-bin.stable."1.98.1".minimal.override {
          extensions = [
            "clippy"
            "rust-analyzer"
            "rust-src"
            "rustfmt"
          ];
          targets = [
            "wasm32-unknown-unknown"
            "wasm32-wasip1"
            "wasm32-wasip2"
          ];
        };
        nightlyToolchain = pkgs.rust-bin.nightly."2026-10-01".minimal.override {
          targets = [ "wasm32-wasip3" ];
        };
        wasiSdk = pkgs.stdenvNoCC.mkDerivation {
          pname = "wasi-sdk";
          version = "34.0";
          src = pkgs.fetchurl {
            url = "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-x86_64-linux.tar.gz";
            hash = "sha256-t2HjoHIduunAmgBZ5f2yv5F9G0qKe0MPs7Wq+wmEssQ=";
          };
          nativeBuildInputs = [ pkgs.autoPatchelfHook ];
          buildInputs = [
            pkgs.stdenv.cc.cc.lib
            pkgs.zlib
            pkgs.libxml2
            pkgs.ncurses
          ];
          installPhase = "cp -R . $out";
        };
        wasmBindgenCliVersion = "0.2.129";
        wasmBindgenCliSource = pkgs.fetchCrate {
          pname = "wasm-bindgen-cli";
          version = wasmBindgenCliVersion;
          hash = "sha256-pcecKQd7E8Opw6bkFoE569epUi7gh5qpQF1e5PJY6V8=";
        };
        wasmBindgenCli = pkgs.buildWasmBindgenCli {
          version = wasmBindgenCliVersion;
          src = wasmBindgenCliSource;
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            pname = "wasm-bindgen-cli-${wasmBindgenCliVersion}";
            version = wasmBindgenCliVersion;
            src = wasmBindgenCliSource;
            hash = "sha256-vmUrWVU7kPJJxO5qIVeAkwQyWDELO1Z4Z5gitz2kco8=";
          };
        };
        browserTools = [
          wasmBindgenCli
          pkgs.nodejs
          pkgs.chromium
          pkgs.chromedriver
        ];
        craneLib = (crane.mkLib pkgs).overrideToolchain stableToolchain;
        cargoSource = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            (craneLib.filterCargoSources path type)
            || pkgs.lib.hasInfix "/tests/data/" path
            || pkgs.lib.hasInfix "/known-failures/" path
            || pkgs.lib.hasSuffix "/crates/papo-conformance/coverage-checklist.toml" path
            || pkgs.lib.hasSuffix "/crates/papo-conformance/coverage-waivers.toml" path
            || pkgs.lib.hasInfix "/crates/papo-conformance/scripts/" path
            || pkgs.lib.hasSuffix "/docs/conformance-coverage.md" path
            || pkgs.lib.hasSuffix "/docs/conformance-matrix.md" path
            || pkgs.lib.hasSuffix "/docs/server-notes.md" path
            || pkgs.lib.hasSuffix "/scripts/conformance-coverage" path
            || pkgs.lib.hasSuffix "/scripts/conformance-matrix" path
            || pkgs.lib.hasSuffix "/scripts/server-notes" path
            || pkgs.lib.hasSuffix "/scripts/test_server_notes.py" path
            || pkgs.lib.hasSuffix "/nix" path
            || pkgs.lib.hasInfix "/nix/" path;
        };
        nixTestRunner = "${pkgs.lib.getExe pkgs.bash} ${cargoSource}/nix/test-runner";
        craneArgs = {
          src = cargoSource;
          strictDeps = true;
          pname = "papo-workspace";
          version = "0.0.0";
        };
        cargoArtifacts = craneLib.buildDepsOnly craneArgs;
        profileConformance =
          profile:
          craneLib.cargoTest (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "-p papo-conformance";
              nativeBuildInputs = [
                pkgs.openssl
                soju
              ];
              PAPO_TEST_PROFILE = profile;
              ERGO_BIN = pkgs.lib.getExe ergochat;
              ERGO_CONFIG = ergoConfig;
              ERGO_LANGUAGES = "${ergochat.src}/languages";
              SOJU_BIN = pkgs.lib.getExe' soju "soju";
              SOJUCTL_BIN = pkgs.lib.getExe' soju "sojuctl";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
            }
          );
        browserCargoVendor = craneLib.vendorCargoDeps {
          src = cargoSource;
          cargoLock = ./crates/papo-core/tests/browser/Cargo.lock;
        };
        browserCraneArgs = craneArgs // {
          pname = "papo-core-browser-tests";
          cargoArtifacts = null;
          cargoVendorDir = browserCargoVendor;
          cargoLock = ./crates/papo-core/tests/browser/Cargo.lock;
        };
        papoTestServer = craneLib.buildPackage (
          craneArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "-p papo-test-server --bin papo-test-server";
            doCheck = false;
          }
        );
        cargoWasip3 = pkgs.writeShellApplication {
          name = "cargo-wasip3";
          text = ''
            export RUSTC=${nightlyToolchain}/bin/rustc
            export RUSTDOC=${nightlyToolchain}/bin/rustdoc
            case "''${1-}" in
              --version|-V) exec ${nightlyToolchain}/bin/cargo "$@" ;;
              *)
                subcommand="''${1:?a cargo subcommand is required}"
                shift
                exec ${nightlyToolchain}/bin/cargo "$subcommand" --target wasm32-wasip3 "$@"
                ;;
            esac
          '';
        };
        ergochat = pkgs.callPackage ./nix/ergochat.nix { };
        unrealircd = pkgs.callPackage ./nix/unrealircd.nix { };
        solanum = pkgs.solanum.overrideAttrs (old: {
          patches = (old.patches or [ ]) ++ [ ./nix/solanum-numeric-dns-port.patch ];
          postPatch = (old.postPatch or "") + ''
            substituteInPlace ircd/bandbi.c --replace-fail \
              'rb_setenv("BANDB_DBPATH", ircd_paths[IRCD_PATH_BANDB], 1);' \
              'if (getenv("BANDB_DBPATH") == NULL) rb_setenv("BANDB_DBPATH", ircd_paths[IRCD_PATH_BANDB], 1);'
          '';
        });
        soju = pkgs.soju.overrideAttrs (old: {
          patches = (old.patches or [ ]) ++ [ ./nix/soju-cap-lines.patch ];
        });
        ergoConfig = pkgs.runCommand "ergo.yaml" { nativeBuildInputs = [ pkgs.yq-go ]; } ''
          yq eval-all '
            (select(fi == 0) | del(.server.listeners, .server.webirc, .opers))
              * select(fi == 1)
            | ... comments = ""
            | . style = ""
          ' ${ergochat.src}/default.yaml ${(pkgs.formats.yaml { }).generate "empty.yaml" { }} > $out
        '';
        cargoHook =
          {
            name,
            text,
            runtimeInputs ? [ ],
            stages ? [ "pre-commit" ],
          }:
          {
            enable = true;
            entry = "${
              pkgs.writeShellApplication {
                inherit name text;
                runtimeInputs = [ stableToolchain ] ++ runtimeInputs;
              }
            }/bin/${name}";
            files = "(^|/)(Cargo\\.(toml|lock)|.*\\.rs)$";
            pass_filenames = false;
            inherit stages;
          };
        offlineHooks = {
          no-merge-commits = {
            enable = true;
            always_run = true;
            pass_filenames = false;
            entry = toString (
              pkgs.writeShellScript "check-no-merge-commits" ''
                if ${pkgs.git}/bin/git rev-parse --verify --quiet refs/remotes/upstream/main >/dev/null \
                  || ${pkgs.git}/bin/git rev-parse --verify --quiet refs/remotes/origin/main >/dev/null \
                  || ${pkgs.git}/bin/git rev-parse --verify --quiet refs/heads/main >/dev/null; then
                  exec ${pkgs.bash}/bin/bash ${./scripts/check-no-merge-commits}
                fi
              ''
            );
          };
          nixfmt.enable = true;
          deadnix = {
            enable = true;
            settings = {
              noLambdaArg = true;
              noLambdaPatternNames = true;
            };
          };
          check-toml.enable = true;
          check-merge-conflicts.enable = true;
          end-of-file-fixer.enable = true;
          trim-trailing-whitespace.enable = true;
          shellcheck.enable = true;
          actionlint.enable = true;
          yamllint.enable = true;
        };
        cargoHooks = {
          rustfmt = cargoHook {
            name = "rustfmt-hook";
            text = "cargo fmt --all -- --check";
          };
          clippy = cargoHook {
            name = "clippy-hook";
            text = ''
              cargo clippy --workspace --all-targets --locked -- -W clippy::pedantic -D warnings
              cargo clippy --workspace --all-targets --all-features --locked -- -W clippy::pedantic -D warnings
            '';
          };
          wasi-checks = cargoHook {
            name = "wasi-checks-hook";
            text = ''
              export CC_wasm32_wasip1=${wasiSdk}/bin/wasm32-wasip1-clang
              export CC_wasm32_wasip2=${wasiSdk}/bin/wasm32-wasip2-clang
              export WASI_SDK_PATH=${wasiSdk}
              cargo check -p papo-proto -p papo-core --all-targets --target wasm32-wasip1 --locked
              cargo check -p papo --example custom_stream --no-default-features --target wasm32-wasip1 --locked
              cargo check -p papo-proto -p papo-core --all-targets --target wasm32-wasip2 --locked
              cargo check -p papo-proto --all-targets --all-features --target wasm32-wasip2 --locked
              cargo check -p papo-proto -p papo-core --target wasm32-unknown-unknown --locked
              cargo check -p papo --target wasm32-unknown-unknown --no-default-features --locked
              cargo check -p papo --target wasm32-unknown-unknown --locked
              cargo check -p papo --target wasm32-unknown-unknown --all-features --locked
              cargo check -p papo -p papo-conformance --target wasm32-wasip2 --locked
              cargo test -p papo-conformance --lib --target wasm32-wasip2 --locked registered_scenarios_are_complete_and_sorted
            '';
          };
          wasip3-checks = cargoHook {
            name = "wasip3-checks-hook";
            text = ''
              export CC_wasm32_wasip3=${wasiSdk}/bin/wasm32-wasip3-clang
              export WASI_SDK_PATH=${wasiSdk}
              ${cargoWasip3}/bin/cargo-wasip3 check -p papo-proto -p papo-core --all-targets --locked
              ${cargoWasip3}/bin/cargo-wasip3 check -p papo-proto --all-targets --all-features --locked
              ${cargoWasip3}/bin/cargo-wasip3 check -p papo -p papo-conformance --locked
              ${cargoWasip3}/bin/cargo-wasip3 test -p papo-conformance --lib --locked registered_scenarios_are_complete_and_sorted
            '';
          };
          cargo-deny = cargoHook {
            name = "cargo-deny-hook";
            runtimeInputs = [ pkgs.cargo-deny ];
            text = "cargo deny check bans licenses sources";
          };
          public-api = cargoHook {
            name = "public-api-hook";
            runtimeInputs = [ pkgs.cargo-public-api ];
            text = ''
              check_api() {
                package=$1
                crate=$2
                features=$3
                expected=$4
                target=$5
                actual=$(mktemp)
                CARGO_INCREMENTAL=0 \
                  RUSTC=${nightlyToolchain}/bin/rustc \
                  RUSTDOC=${nightlyToolchain}/bin/rustdoc \
                  ${nightlyToolchain}/bin/cargo rustdoc \
                    -p "$package" --no-default-features --features "$features" --lib --target-dir "$target" \
                    -- -Z unstable-options --output-format json
                cargo public-api \
                  --rustdoc-json "$target/doc/$crate.json" \
                  --color=never -s | bash nix/normalize-public-api.sh > "$actual"
                if [[ "$package" == papo ]] && grep -Eq 'papo_(core|proto)::' "$actual"; then
                  echo "public API contains a foreign papo-core or papo-proto path" >&2
                  grep -En 'papo_(core|proto)::' "$actual" >&2
                  return 1
                fi
                diff -u "$expected" "$actual"
                rm -f "$actual"
              }
              stable=bot,sts-file,websocket,tls-aws-lc,tls-ring
              check_api papo-proto papo_proto "" crates/papo-proto/public-api.txt target/public-api/papo-proto
              check_api papo-core papo_core "" crates/papo-core/public-api.txt target/public-api/papo-core-stable
              check_api papo-core papo_core unstable crates/papo-core/public-api-unstable.txt target/public-api/papo-core-unstable
              check_api papo papo "$stable" crates/papo/public-api.txt target/public-api/papo-stable
              check_api papo papo "$stable,unstable" crates/papo/public-api-unstable.txt target/public-api/papo-unstable
            '';
          };
          plain-tests = cargoHook {
            name = "plain-tests-hook";
            runtimeInputs = [ pkgs.cargo-nextest ];
            text = ''
              cargo nextest run --workspace --all-features --locked -E 'not binary(/^live_/)'
            '';
          };
          cargo-nextest = cargoHook {
            name = "cargo-nextest-hook";
            runtimeInputs = [ pkgs.cargo-nextest ];
            text = ''
              cargo nextest run --workspace --locked
              cargo nextest run --workspace --all-features --locked
            '';
            stages = [ "pre-push" ];
          };
          browser-tests = cargoHook {
            name = "browser-tests-hook";
            runtimeInputs = browserTools;
            text = ''
              CARGO_TARGET_DIR="$PWD/target" \
                ${pkgs.lib.getExe pkgs.bash} "$PWD/nix/test-runner" --browser
            '';
            stages = [ "pre-push" ];
          };
          wasip2-tests = cargoHook {
            name = "wasip2-tests-hook";
            runtimeInputs = [ pkgs.wasmtime ];
            text = ''
              cargo test --workspace --locked --target wasm32-wasip2
              cargo test -p papo-proto --all-features --locked --target wasm32-wasip2
            '';
            stages = [ "pre-push" ];
          };
          wasip3-tests = cargoHook {
            name = "wasip3-tests-hook";
            runtimeInputs = [ pkgs.wasmtime ];
            text = ''
              ${cargoWasip3}/bin/cargo-wasip3 test --workspace --locked
              ${cargoWasip3}/bin/cargo-wasip3 test -p papo-proto --all-features --locked
            '';
            stages = [ "pre-push" ];
          };
          doctests = cargoHook {
            name = "doctests-hook";
            text = "cargo test --doc --workspace --locked";
            stages = [ "pre-push" ];
          };
          docs = cargoHook {
            name = "docs-hook";
            runtimeInputs = [ pkgs.ripgrep ];
            text = ''
              RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
              if rg --only-matching --multiline --multiline-dotall --glob '*.html' \
                '<pre class="[^"]*rust-example-rendered[^"]*">.*?</pre>' target/doc/papo \
                | rg --line-number 'papo_core::'; then
                echo "papo rustdoc examples must use papo paths" >&2
                exit 1
              fi
            '';
            stages = [ "pre-push" ];
          };
          fuzz = cargoHook {
            name = "fuzz-hook";
            runtimeInputs = [
              pkgs.cargo-fuzz
              nightlyToolchain
            ];
            text = ''
              cd fuzz
              RUSTC=${nightlyToolchain}/bin/rustc ${nightlyToolchain}/bin/cargo fuzz run line -- -max_total_time=30 -verbosity=0
              RUSTC=${nightlyToolchain}/bin/rustc ${nightlyToolchain}/bin/cargo fuzz run construction -- -max_total_time=30 -verbosity=0
            '';
            stages = [ "pre-push" ];
          };
        };
        gitHooks = git-hooks.lib.${system}.run {
          src = ./.;
          hooks = offlineHooks // cargoHooks;
        };
      in
      {
        checks = {
          no-merge-commits = pkgs.runCommand "test-no-merge-commits" { nativeBuildInputs = [ pkgs.git ]; } ''
            ${pkgs.bash}/bin/bash ${./scripts/test_no_merge_commits.sh} ${./scripts/check-no-merge-commits}
            touch $out
          '';
          pre-commit = git-hooks.lib.${system}.run {
            src = ./.;
            hooks = offlineHooks;
          };
          clippy = craneLib.cargoClippy (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-targets -- -W clippy::pedantic -D warnings";
            }
          );
          fmt = craneLib.cargoFmt craneArgs;
          doc = craneLib.cargoDoc (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoDocExtraArgs = "--workspace --no-deps";
              env.RUSTDOCFLAGS = "-D warnings";
            }
          );
          nextest = craneLib.cargoNextest (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoExtraArgs = "--workspace --locked";
              cargoNextestPartitionsExtraArgs = "--no-tests=pass";
              nativeBuildInputs = [
                pkgs.openssl
                pkgs.atheme
                pkgs.inspircd
                soju
                solanum
                unrealircd
              ];
              ERGO_BIN = pkgs.lib.getExe ergochat;
              ERGO_CONFIG = ergoConfig;
              ERGO_LANGUAGES = "${ergochat.src}/languages";
              SOJU_BIN = pkgs.lib.getExe' soju "soju";
              SOJUCTL_BIN = pkgs.lib.getExe' soju "sojuctl";
              ATHEME_BIN = pkgs.lib.getExe' pkgs.atheme "atheme-services";
              ATHEME_MODULES = "${pkgs.atheme}/modules";
              INSPIRCD_BIN = pkgs.lib.getExe' pkgs.inspircd "inspircd";
              INSPIRCD_MODULES = "${pkgs.lib.getLib pkgs.inspircd}/lib/inspircd";
              SOLANUM_BIN = pkgs.lib.getExe solanum;
              SOLANUM_MODULES = "${solanum}/lib/solanum/modules";
              UNREALIRCD_BIN = pkgs.lib.getExe' unrealircd "unrealircd";
              UNREALIRCD_CONFIG = "${unrealircd}/share/unrealircd/conf";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
              CARGO_TARGET_WASM32_WASIP1_RUNNER = nixTestRunner;
              CARGO_TARGET_WASM32_WASIP2_RUNNER = nixTestRunner;
              CARGO_TARGET_WASM32_WASIP3_RUNNER = nixTestRunner;
            }
          );
          browser-tests = craneLib.mkCargoDerivation (
            browserCraneArgs
            // {
              nativeBuildInputs = browserTools ++ [ pkgs.openssl ];
              PAPO_TEST_SERVER_BIN = pkgs.lib.getExe' papoTestServer "papo-test-server";
              ERGO_BIN = pkgs.lib.getExe ergochat;
              ERGO_CONFIG = ergoConfig;
              ERGO_LANGUAGES = "${ergochat.src}/languages";
              doInstallCargoArtifacts = false;
              buildPhaseCargoCommand = "${nixTestRunner} --browser";
              installPhaseCommand = "touch $out";
            }
          );
          browser-size = craneLib.mkCargoDerivation (
            browserCraneArgs
            // {
              nativeBuildInputs = [ pkgs.binaryen ];
              doInstallCargoArtifacts = false;
              buildPhaseCargoCommand = ''
                cargo build \
                  --manifest-path crates/papo-core/tests/browser/Cargo.toml \
                  --target wasm32-unknown-unknown --target-dir target/browser-size \
                  --release --locked
                wasm-opt -Oz \
                  target/browser-size/wasm32-unknown-unknown/release/papo_core_browser_tests.wasm \
                  -o papo-core-probe.wasm
                size=$(stat --format=%s papo-core-probe.wasm)
                budget=867086
                echo "optimized papo-core browser probe: $size bytes (budget: $budget bytes)"
                if (( size > budget )); then
                  echo "papo-core browser probe exceeds its size budget" >&2
                  exit 1
                fi
                cargo build \
                  --manifest-path crates/papo-core/tests/browser/Cargo.toml \
                  --target wasm32-unknown-unknown --target-dir target/browser-driver-size \
                  --release --features driver --locked
                wasm-opt -Oz \
                  target/browser-driver-size/wasm32-unknown-unknown/release/papo_core_browser_tests.wasm \
                  -o papo-driver-probe.wasm
                size=$(stat --format=%s papo-driver-probe.wasm)
                budget=1330000
                echo "optimized papo browser driver: $size bytes (recorded 1208550 on 2026-10-18; budget: $budget bytes)"
                if (( size > budget )); then
                  echo "papo browser driver exceeds its size budget" >&2
                  exit 1
                fi
              '';
              installPhaseCommand = ''
                mkdir -p $out
                cp papo-core-probe.wasm $out/
                cp papo-driver-probe.wasm $out/
              '';
            }
          );
          conformance-unrealircd = craneLib.cargoTest (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "-p papo-conformance";
              nativeBuildInputs = [
                pkgs.openssl
                pkgs.atheme
                unrealircd
              ];
              PAPO_TEST_SERVER = "unrealircd";
              ATHEME_BIN = pkgs.lib.getExe' pkgs.atheme "atheme-services";
              ATHEME_MODULES = "${pkgs.atheme}/modules";
              UNREALIRCD_BIN = pkgs.lib.getExe' unrealircd "unrealircd";
              UNREALIRCD_CONFIG = "${unrealircd}/share/unrealircd/conf";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
            }
          );
          conformance-inspircd = craneLib.cargoTest (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "-p papo-conformance";
              nativeBuildInputs = [
                pkgs.openssl
                pkgs.atheme
                pkgs.inspircd
              ];
              PAPO_TEST_SERVER = "inspircd";
              ATHEME_BIN = pkgs.lib.getExe' pkgs.atheme "atheme-services";
              ATHEME_MODULES = "${pkgs.atheme}/modules";
              INSPIRCD_BIN = pkgs.lib.getExe' pkgs.inspircd "inspircd";
              INSPIRCD_MODULES = "${pkgs.lib.getLib pkgs.inspircd}/lib/inspircd";
              CARGO_INCREMENTAL = "0";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
            }
          );
          conformance-solanum = craneLib.cargoTest (
            craneArgs
            // {
              inherit cargoArtifacts;
              cargoTestExtraArgs = "-p papo-conformance";
              nativeBuildInputs = [
                pkgs.openssl
                pkgs.atheme
                solanum
              ];
              PAPO_TEST_SERVER = "solanum";
              ATHEME_BIN = pkgs.lib.getExe' pkgs.atheme "atheme-services";
              ATHEME_MODULES = "${pkgs.atheme}/modules";
              SOLANUM_BIN = pkgs.lib.getExe solanum;
              SOLANUM_MODULES = "${solanum}/lib/solanum/modules";
              CARGO_INCREMENTAL = "0";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
            }
          );
          profile-ergo-default = profileConformance "ergo-default";
          profile-ergo-persistent = profileConformance "ergo-persistent";
          profile-soju-ergo = profileConformance "soju-ergo";
          conformance-coverage = craneLib.mkCargoDerivation (
            craneArgs
            // {
              inherit cargoArtifacts;
              pname = "papo-conformance-coverage";
              nativeBuildInputs = [ pkgs.python3 ];
              buildPhaseCargoCommand = ''
                python scripts/conformance-coverage --output generated-conformance-coverage.md
                cmp docs/conformance-coverage.md generated-conformance-coverage.md
              '';
              installPhaseCommand = "mkdir -p $out";
            }
          );
          conformance-matrix = craneLib.mkCargoDerivation (
            craneArgs
            // {
              inherit cargoArtifacts;
              pname = "papo-conformance-matrix";
              nativeBuildInputs = [
                pkgs.openssl
                pkgs.atheme
                pkgs.inspircd
                pkgs.python3
                soju
                solanum
                unrealircd
              ];
              ERGO_BIN = pkgs.lib.getExe ergochat;
              ERGO_CONFIG = ergoConfig;
              ERGO_LANGUAGES = "${ergochat.src}/languages";
              SOJU_BIN = pkgs.lib.getExe' soju "soju";
              SOJUCTL_BIN = pkgs.lib.getExe' soju "sojuctl";
              ATHEME_BIN = pkgs.lib.getExe' pkgs.atheme "atheme-services";
              ATHEME_MODULES = "${pkgs.atheme}/modules";
              INSPIRCD_BIN = pkgs.lib.getExe' pkgs.inspircd "inspircd";
              INSPIRCD_MODULES = "${pkgs.lib.getLib pkgs.inspircd}/lib/inspircd";
              SOLANUM_BIN = pkgs.lib.getExe solanum;
              SOLANUM_MODULES = "${solanum}/lib/solanum/modules";
              UNREALIRCD_BIN = pkgs.lib.getExe' unrealircd "unrealircd";
              UNREALIRCD_CONFIG = "${unrealircd}/share/unrealircd/conf";
              CARGO_INCREMENTAL = "0";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
              buildPhaseCargoCommand = ''
                python scripts/conformance-matrix --output generated-conformance-matrix.md
                cmp docs/conformance-matrix.md generated-conformance-matrix.md
              '';
              installPhaseCommand = "mkdir -p $out";
            }
          );
          server-notes = craneLib.mkCargoDerivation (
            craneArgs
            // {
              inherit cargoArtifacts;
              pname = "papo-server-notes";
              nativeBuildInputs = [ pkgs.python3 ];
              buildPhaseCargoCommand = ''
                python scripts/test_server_notes.py
                python scripts/server-notes --output generated-server-notes.md
                cmp docs/server-notes.md generated-server-notes.md
              '';
              installPhaseCommand = "mkdir -p $out";
            }
          );
        };

        devShells.default = pkgs.mkShell {
          packages = [
            stableToolchain
            cargoWasip3
            pkgs.wasmtime
            pkgs.wasm-tools
            pkgs.binaryen
            pkgs.cargo-nextest
            pkgs.cargo-deny
            pkgs.cargo-public-api
            pkgs.cargo-mutants
            pkgs.cargo-fuzz
            pkgs.cargo-llvm-cov
            wasiSdk
            pkgs.cmake
            pkgs.ninja
            pkgs.perl
            pkgs.openssl
            pkgs.taplo
            pkgs.git-absorb
            pkgs.atheme
            pkgs.inspircd
            solanum
            soju
            ergochat
            unrealircd
            nightlyToolchain
          ]
          ++ browserTools
          ++ gitHooks.enabledPackages;
          ERGO_BIN = pkgs.lib.getExe ergochat;
          ERGO_CONFIG = ergoConfig;
          ERGO_LANGUAGES = "${ergochat.src}/languages";
          SOJU_BIN = pkgs.lib.getExe' soju "soju";
          SOJUCTL_BIN = pkgs.lib.getExe' soju "sojuctl";
          ATHEME_BIN = pkgs.lib.getExe' pkgs.atheme "atheme-services";
          ATHEME_MODULES = "${pkgs.atheme}/modules";
          INSPIRCD_BIN = pkgs.lib.getExe' pkgs.inspircd "inspircd";
          INSPIRCD_MODULES = "${pkgs.lib.getLib pkgs.inspircd}/lib/inspircd";
          SOLANUM_BIN = pkgs.lib.getExe solanum;
          SOLANUM_MODULES = "${solanum}/lib/solanum/modules";
          UNREALIRCD_BIN = pkgs.lib.getExe' unrealircd "unrealircd";
          UNREALIRCD_CONFIG = "${unrealircd}/share/unrealircd/conf";
          CC_wasm32_wasip1 = "${wasiSdk}/bin/wasm32-wasip1-clang";
          CC_wasm32_wasip2 = "${wasiSdk}/bin/wasm32-wasip2-clang";
          CC_wasm32_wasip3 = "${wasiSdk}/bin/wasm32-wasip3-clang";
          WASI_SDK_PATH = wasiSdk;
          CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
          CARGO_TARGET_WASM32_WASIP1_RUNNER = nixTestRunner;
          CARGO_TARGET_WASM32_WASIP2_RUNNER = nixTestRunner;
          CARGO_TARGET_WASM32_WASIP3_RUNNER = nixTestRunner;
          inherit (gitHooks) shellHook;
        };
      }
    );
}
