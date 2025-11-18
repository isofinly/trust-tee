{
  description = "Rust project";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ rust-overlay.overlays.default ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.selectLatestNightlyWith (
          toolchain:
          toolchain.default.override {
            extensions = [
              "rust-src"
              "rust-analyzer"
              "miri"
            ];
          }
        );

      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
          ];

          shellHook = ''
            echo ""
            echo "Tools:"
            echo "  Rust:    $(rustc --version)"
            echo "  Miri:    $(cargo miri --version)"

            export RUST_BACKTRACE=1
            export CARGO_HOME="$HOME/.cargo"

          '';
        };
      }
    );
}
