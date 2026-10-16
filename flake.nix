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
        craneLib = (crane.mkLib pkgs).overrideToolchain stableToolchain;
        cargoSource = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            (craneLib.filterCargoSources path type)
            || pkgs.lib.hasInfix "/tests/data/" path
            || pkgs.lib.hasInfix "/known-failures/" path
            || pkgs.lib.hasInfix "/crates/papo-conformance/scripts/" path
            || pkgs.lib.hasSuffix "/docs/conformance-matrix.md" path
            || pkgs.lib.hasSuffix "/scripts/conformance-matrix" path
            || pkgs.lib.hasSuffix "/nix" path
            || pkgs.lib.hasSuffix "/nix/test-runner" path;
        };
        nixTestRunner = "${pkgs.lib.getExe pkgs.bash} ${cargoSource}/nix/test-runner";
        craneArgs = {
          src = cargoSource;
          strictDeps = true;
          pname = "papo-workspace";
          version = "0.0.0";
        };
        cargoArtifacts = craneLib.buildDepsOnly craneArgs;
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
            text = "cargo clippy --workspace --all-targets --locked -- -W clippy::pedantic -D warnings";
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
              cargo check -p papo -p papo-conformance --target wasm32-wasip2 --locked
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
              actual=$(mktemp)
              trap 'rm -f "$actual"' EXIT
              CARGO_INCREMENTAL=0 \
                RUSTC=${nightlyToolchain}/bin/rustc \
                RUSTDOC=${nightlyToolchain}/bin/rustdoc \
                ${nightlyToolchain}/bin/cargo rustdoc \
                  -p papo --all-features --lib --target-dir target/public-api \
                  -- -Z unstable-options --output-format json
              cargo public-api \
                --rustdoc-json target/public-api/doc/papo.json \
                --color=never -sss > "$actual"
              diff -u crates/papo/public-api.txt "$actual"
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
              cargo nextest run -p papo-proto --features precis --locked
              cargo nextest run -p papo --lib --all-features --locked
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
            text = ''
              RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
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
              RUSTC=${nightlyToolchain}/bin/rustc ${nightlyToolchain}/bin/cargo fuzz run message -- -max_total_time=30 -verbosity=0
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
              nativeBuildInputs = [ pkgs.openssl ];
              ERGO_BIN = pkgs.lib.getExe ergochat;
              ERGO_CONFIG = ergoConfig;
              ERGO_LANGUAGES = "${ergochat.src}/languages";
              CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER = nixTestRunner;
              CARGO_TARGET_WASM32_WASIP1_RUNNER = nixTestRunner;
              CARGO_TARGET_WASM32_WASIP2_RUNNER = nixTestRunner;
              CARGO_TARGET_WASM32_WASIP3_RUNNER = nixTestRunner;
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
                solanum
                unrealircd
              ];
              ERGO_BIN = pkgs.lib.getExe ergochat;
              ERGO_CONFIG = ergoConfig;
              ERGO_LANGUAGES = "${ergochat.src}/languages";
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
        };

        devShells.default = pkgs.mkShell {
          packages = [
            stableToolchain
            cargoWasip3
            pkgs.wasmtime
            pkgs.wasm-tools
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
            ergochat
            unrealircd
            nightlyToolchain
          ]
          ++ gitHooks.enabledPackages;
          ERGO_BIN = pkgs.lib.getExe ergochat;
          ERGO_CONFIG = ergoConfig;
          ERGO_LANGUAGES = "${ergochat.src}/languages";
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
