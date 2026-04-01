{
  description = "IsoNim-GPUI — Nim bindings for GPUI, Zed's GPU-accelerated UI framework";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        fenixPkgs = fenix.packages.${system};
        rustToolchain = fenixPkgs.stable.withComponents [
          "cargo"
          "clippy"
          "rustc"
          "rust-src"
          "rust-std"
          "rustfmt"
        ];
        isLinux = pkgs.lib.hasSuffix "linux" system;
      in
      {
        devShells.default = pkgs.mkShell {
          packages =
            [
              rustToolchain
              pkgs.nim
              pkgs.nimble
              pkgs.just
              pkgs.pkg-config
              pkgs.openssl
              pkgs.cmake
              pkgs.clang
              pkgs.protobuf
            ]
            ++ pkgs.lib.optionals isLinux [
              # GPU / rendering
              pkgs.fontconfig
              pkgs.freetype
              pkgs.libGL
              pkgs.libxkbcommon
              pkgs.vulkan-loader

              # Wayland
              pkgs.wayland
              pkgs.wayland-protocols

              # X11
              pkgs.libx11
              pkgs.libxcursor
              pkgs.libxi
              pkgs.libxrandr
              pkgs.libxcb

              # GPUI additional deps
              pkgs.sqlite
              pkgs.zlib
              pkgs.curl
              pkgs.libgit2
              pkgs.alsa-lib
            ];

          # Ensure the linker can find native libs at build time
          LD_LIBRARY_PATH = pkgs.lib.optionalString isLinux (
            pkgs.lib.makeLibraryPath [
              pkgs.fontconfig
              pkgs.freetype
              pkgs.libGL
              pkgs.libxkbcommon
              pkgs.vulkan-loader
              pkgs.wayland
              pkgs.libx11
              pkgs.libxcursor
              pkgs.libxi
              pkgs.libxrandr
              pkgs.libxcb
              pkgs.sqlite
              pkgs.zlib
              pkgs.curl
              pkgs.libgit2
              pkgs.alsa-lib
            ]
          );

          shellHook = ''
            echo "isonim-gpui dev shell — rust $(rustc --version), nim $(nim --version 2>&1 | head -1)"
          '';
        };
      }
    );
}
