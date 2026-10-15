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
      nixpkgs,
      flake-utils,
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
        cargoWasip3 = pkgs.writeShellApplication {
          name = "cargo-wasip3";
          text = ''
            export RUSTC=${nightlyToolchain}/bin/rustc
            export RUSTDOC=${nightlyToolchain}/bin/rustdoc
            case "''${1-}" in
              --version|-V) exec ${nightlyToolchain}/bin/cargo "$@" ;;
              *) exec ${nightlyToolchain}/bin/cargo "$@" --target wasm32-wasip3 ;;
            esac
          '';
        };
        ergochat = pkgs.callPackage ./nix/ergochat.nix { };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            stableToolchain
            cargoWasip3
            pkgs.wasmtime
            pkgs.wasm-tools
            pkgs.cargo-nextest
            pkgs.cargo-deny
            pkgs.cargo-mutants
            pkgs.cargo-llvm-cov
            pkgs.taplo
            pkgs.git-absorb
            ergochat
          ];
          ERGO_BIN = pkgs.lib.getExe ergochat;
          CARGO_TARGET_WASM32_WASIP1_RUNNER = "wasmtime run";
          CARGO_TARGET_WASM32_WASIP2_RUNNER =
            "wasmtime run -S inherit-network -S allow-ip-name-lookup";
          CARGO_TARGET_WASM32_WASIP3_RUNNER =
            "wasmtime run -S inherit-network -S allow-ip-name-lookup -S p3";
        };
      }
    );
}
