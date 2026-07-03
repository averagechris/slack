{
  description = "Nix flake for slack";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {inherit system;};
        lib = pkgs.lib;
        cargoToml = fromTOML (builtins.readFile ./Cargo.toml);
        cliConfig = fromTOML (builtins.readFile ./config/cli.toml);
        package = cargoToml.package;
        cliProgram = cliConfig.program_name;
        # Map Cargo SPDX-ish license strings to nixpkgs license attrs.
        licenseFor = licenseString:
          if licenseString == "MIT OR Apache-2.0"
          then [lib.licenses.mit lib.licenses.asl20]
          else if licenseString == "MIT"
          then lib.licenses.mit
          else if licenseString == "Apache-2.0"
          then lib.licenses.asl20
          else null;
        commonRustArgs = {
          version = package.version;
          src = lib.cleanSource ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          # Platform-specific runtime library deps. The crate is expected to
          # gain a `keyring` dependency (Linux secret-service over dbus;
          # macOS Security framework is provided by the Darwin SDK stubs in
          # current nixpkgs). Extend the lists below per platform as needed.
          buildInputs =
            lib.optionals pkgs.stdenv.isLinux (with pkgs; [
              dbus
            ])
            ++ lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
              # Darwin frameworks (Security, SystemConfiguration, ...) are
              # provided automatically by the SDK in current nixpkgs.
            ]);
        };
        fetchUpstreamScript = ''
          # Refresh GitHub upstream refs in both git and jj. For jj, Git's
          # refs/remotes/upstream/<branch> is exposed as the remote bookmark
          # <branch>@upstream.
          exec jj git fetch --remote upstream "$@"
        '';
        ciFmtScript = ''
          cargo fmt --all --check
          alejandra --check flake.nix
        '';
        ciClippyScript = ''
          cargo clippy --locked --all-targets -- -D warnings
        '';
        ciTestScript = ''
          cargo test --locked
        '';
        mkRepoScript = {
          name,
          runtimeInputs ? [],
          text,
        }:
          pkgs.writeShellApplication {
            inherit name runtimeInputs;
            inherit text;
          };
        slack = pkgs.rustPlatform.buildRustPackage (commonRustArgs
          // {
            pname = cliProgram;
            doCheck = false;

            meta = lib.attrsets.filterAttrs (_: value: value != null) {
              description = package.description or null;
              homepage = package.homepage or package.repository or null;
              license = licenseFor (package.license or null);
              mainProgram = cliProgram;
            };
          });
        fetch-upstream = mkRepoScript {
          name = "fetch-upstream";
          text = fetchUpstreamScript;
          runtimeInputs = with pkgs; [
            jujutsu
          ];
        };
        ci-fmt = mkRepoScript {
          name = "ci-fmt";
          text = ciFmtScript;
          runtimeInputs = with pkgs; [
            alejandra
            cargo
            rustfmt
          ];
        };
        ci-clippy = mkRepoScript {
          name = "ci-clippy";
          text = ciClippyScript;
          runtimeInputs = with pkgs; [
            cargo
            clippy
            rustc
          ];
        };
        ci-test = mkRepoScript {
          name = "ci-test";
          text = ciTestScript;
          runtimeInputs = with pkgs; [
            cargo
            rustc
          ];
        };
        repo-scripts = pkgs.symlinkJoin {
          name = "${package.name}-scripts";
          paths = [
            ci-clippy
            ci-fmt
            ci-test
            fetch-upstream
          ];
        };
        fmt-check =
          pkgs.runCommand "${package.name}-fmt-check" {
            nativeBuildInputs = [ci-fmt];
            src = lib.cleanSource ./.;
          } ''
            export HOME="$TMPDIR"
            cp -R "$src" source
            chmod -R +w source
            cd source
            ci-fmt
            mkdir -p "$out"
          '';
        # `nix fmt` invokes the formatter app without path arguments. Alejandra
        # treats no arguments as "format stdin", which fails on empty stdin, so
        # keep the formatter as Alejandra but default it to formatting the repo.
        nix-formatter = pkgs.writeShellApplication {
          name = "alejandra";
          runtimeInputs = with pkgs; [
            alejandra
          ];
          text = ''
            if [[ $# -eq 0 ]]; then
              exec alejandra .
            fi

            exec alejandra "$@"
          '';
        };
      in {
        formatter = nix-formatter;

        packages = {
          default = slack;
          slack = slack;
          ci-clippy = ci-clippy;
          ci-fmt = ci-fmt;
          ci-test = ci-test;
          fetch-upstream = fetch-upstream;
          scripts = repo-scripts;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = slack;
          exePath = "/bin/${cliProgram}";
        };
        apps.slack = flake-utils.lib.mkApp {
          drv = slack;
          exePath = "/bin/${cliProgram}";
        };
        apps.ci-clippy = flake-utils.lib.mkApp {
          drv = ci-clippy;
        };
        apps.ci-fmt = flake-utils.lib.mkApp {
          drv = ci-fmt;
        };
        apps.ci-test = flake-utils.lib.mkApp {
          drv = ci-test;
        };
        apps.fetch-upstream = flake-utils.lib.mkApp {
          drv = fetch-upstream;
        };

        checks = {
          build = slack;
          fmt = fmt-check;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [slack];
          packages = with pkgs; [
            alejandra
            cargo
            cargo-audit
            cargo-deny
            cargo-edit
            cargo-machete
            cargo-nextest
            cargo-outdated
            clippy
            jujutsu
            nixd
            pkg-config
            rust-analyzer
            rustc
            rustfmt
            repo-scripts
          ];
        };
      }
    )
    // {
      overlays.default = final: prev: {
        inherit (self.packages.${prev.system}) slack;
      };
    };
}
