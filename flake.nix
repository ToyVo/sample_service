{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay.url = "github:oxalica/rust-overlay";
    treefmt-nix.url = "github:numtide/treefmt-nix";
  };

  outputs =
    inputs@{
      nixpkgs,
      flake-parts,
      rust-overlay,
      treefmt-nix,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      imports = [
        treefmt-nix.flakeModule
      ];

      perSystem =
        {
          system,
          pkgs,
          lib,
          ...
        }:
        let
          # wasm-bindgen-cli must match the `wasm-bindgen` crate version in Cargo.lock
          # exactly (currently 0.2.128). nixpkgs only ships 0.2.127, so build our own
          # via `buildWasmBindgenCli`.
          wasm-bindgen-cli_0_2_128 = pkgs.buildWasmBindgenCli rec {
            src = pkgs.fetchCrate {
              pname = "wasm-bindgen-cli";
              version = "0.2.128";
              hash = "sha256-a7lcXJnnZkYReja+iUO7NqqrWyv3toxnUgQb8s4IS5s=";
            };
            cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
              inherit src;
              inherit (src) pname version;
              hash = "sha256-R1Tas33Ursy8kqsxguAkG0ZhNed2n5uFTAhw1l2qlLY=";
            };
          };
          cargo-run = pkgs.rustPlatform.buildRustPackage rec {
            pname = "cargo-run";
            version = "0.6.0";

            src = pkgs.fetchCrate {
              inherit pname version;
              hash = "sha256-Wv8V08sTsNmpzZSKAYkvwGDsd3lVLMK1m9iAvSU7HO0=";
            };

            cargoHash = "sha256-ps0ICgMSxxsZRtYrhC7aKgYvXVEjkNdvN1vDeXZozXc=";

            meta = with pkgs.lib; {
              description = "CLI tool for managing project scripts in Rust (provides `cargo-script` and `cgs`)";
              homepage = "https://github.com/rsaz/cargo-script";
              license = licenses.mit;
              mainProgram = "cargo-script";
            };
          };
          openapi-to-rust = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
            pname = "openapi-to-rust";
            version = "0.16.0";

            src = pkgs.fetchFromGitHub {
              owner = "gpu-cli";
              repo = "openapi-to-rust";
              rev = "v${finalAttrs.version}";
              hash = "sha256-BrnmaTiXO6z+tTofswr6fRvDL9vRWiCb1AMyifHwDhU=";
            };

            cargoHash = "sha256-Scp7PFE3ec69X7F1qi93QBzAR5p9qSjD8/NCeepgb/Q=";

            nativeBuildInputs = with pkgs; [
              pkg-config
              cmake
            ];

            # Upstream tests generate throwaway crates that fetch deps from
            # crates.io at test time, which fails in the offline cargo-vendor sandbox.
            doCheck = false;

            meta = with pkgs.lib; {
              description = "Generate typed Rust models, HTTP/SSE clients, and Axum servers from OpenAPI 3.0/3.1";
              homepage = "https://github.com/gpu-cli/openapi-to-rust";
              license = licenses.mit;
              mainProgram = "openapi-to-rust";
            };
          });
        in
        {
          _module.args.pkgs = import nixpkgs {
            inherit system;
            overlays = [
              rust-overlay.overlays.default
            ];
            config.allowUnfree = true;
          };

          treefmt = {
            programs = {
              nixfmt.enable = true;
              rustfmt.enable = true;
              prettier.enable = true;
              toml-sort = {
                all = true;
                enable = true;
              };
            };
          };

          packages = {
            inherit cargo-run openapi-to-rust wasm-bindgen-cli_0_2_128;
          };

          devShells.default = pkgs.mkShell {
            shellHook = ''
              export BINARYEN_CORES=1
              export RUST_LOG="debug"
              export RUST_SRC_PATH=${pkgs.rustPlatform.rustLibSrc}
              export SQLITE3_INCLUDE_DIR="${pkgs.sqlite.dev}/include"
              export SQLITE3_LIB_DIR="${pkgs.sqlite.out}/lib"
            '';
            # `libiconv` provides `-liconv` needed by `ring`/`aws-lc-sys`
            # (via reqwest/rustls). stdenv already supplies the Apple SDK
            # frameworks (Security, SystemConfiguration, CoreFoundation).
            buildInputs =
              with pkgs;
              [ sqlite ]
              ++ lib.optionals stdenv.hostPlatform.isDarwin [
                libiconv
              ];
            nativeBuildInputs = with pkgs; [
              (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
              bacon
              binaryen
              cargo-audit
              cargo-generate
              cargo-llvm-cov
              cargo-nextest
              cargo-run
              cargo-seek
              cmake
              diesel-cli
              diesel-cli-ext
              dioxus-cli
              httpyac
              libiconv
              libpq
              mkcert
              nss.tools
              openapi-to-rust
              openssl
              pkg-config
              postgresql
              rustPlatform.bindgenHook
              secretspec
              wasm-bindgen-cli_0_2_128
            ];
          };
        };
    };
}
