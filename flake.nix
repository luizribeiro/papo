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
        craneLib = (crane.mkLib pkgs).overrideToolchain stableToolchain;
        cargoSource = craneLib.cleanCargoSource ./.;
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
              cargo check -p papo-proto -p papo-core --all-targets --target wasm32-wasip1 --locked
              cargo check -p papo-proto -p papo-core --all-targets --target wasm32-wasip2 --locked
              cargo check -p papo-proto --all-targets --all-features --target wasm32-wasip2 --locked
              cargo check -p papo-proto -p papo-core --target wasm32-unknown-unknown --locked
              cargo check -p papo --target wasm32-wasip2 --locked
            '';
          };
          wasip3-checks = cargoHook {
            name = "wasip3-checks-hook";
            text = ''
              ${cargoWasip3}/bin/cargo-wasip3 check -p papo-proto -p papo-core --all-targets --locked
              ${cargoWasip3}/bin/cargo-wasip3 check -p papo-proto --all-targets --all-features --locked
              ${cargoWasip3}/bin/cargo-wasip3 check -p papo --locked
            '';
          };
          cargo-deny = cargoHook {
            name = "cargo-deny-hook";
            runtimeInputs = [ pkgs.cargo-deny ];
            text = "cargo deny check bans licenses sources";
          };
          cargo-nextest = cargoHook {
            name = "cargo-nextest-hook";
            runtimeInputs = [ pkgs.cargo-nextest ];
            text = ''
              cargo nextest run --workspace --locked
              cargo nextest run -p papo-proto --features precis --locked
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
              ERGO_BIN = pkgs.lib.getExe ergochat;
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
            pkgs.cargo-mutants
            pkgs.cargo-fuzz
            pkgs.cargo-llvm-cov
            pkgs.taplo
            pkgs.git-absorb
            ergochat
            nightlyToolchain
          ]
          ++ gitHooks.enabledPackages;
          ERGO_BIN = pkgs.lib.getExe ergochat;
          CARGO_TARGET_WASM32_WASIP1_RUNNER = "wasmtime run --dir=.";
          CARGO_TARGET_WASM32_WASIP2_RUNNER = "wasmtime run --dir=. -S inherit-network -S allow-ip-name-lookup";
          CARGO_TARGET_WASM32_WASIP3_RUNNER = "wasmtime run --dir=. -S inherit-network -S allow-ip-name-lookup -S p3";
          inherit (gitHooks) shellHook;
        };
      }
    );
}
