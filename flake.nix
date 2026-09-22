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
              # PLAT-19: `ci/run-suite.sh` runs the trap-13 assertion-helper
              # sweep, which is a python3 script. Without this the lane's
              # gate exits 127 and the sweep is silently never taken.
              pkgs.python3
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

              # Headless GUI testing
              pkgs.xorg.xorgserver
              pkgs.xorg.xdpyinfo
              pkgs.mesa
              pkgs.libglvnd
              pkgs.sway
              pkgs.wayland-utils
              pkgs.wf-recorder
              # RS-M14b: `scripts/wayland-capture-frame.sh` reads the
              # compositor output back with grim (via
              # `zwlr_screencopy_manager_v1`), which is what the windowed
              # pixel case in `tests/test_gui.nim` asserts on. It was
              # resolving from the host PATH on this workstation and was
              # absent from the shell, so CI would have been the first
              # place to find out.
              pkgs.grim
              # PLAT-38: `wtype` is how a REAL key reaches a GPUI window
              # here. It is a Wayland client speaking
              # `zwp_virtual_keyboard_manager_v1`, which wlroots (and
              # therefore sway) implements, so the key it sends is
              # attached to the compositor's own `wl_seat` and is routed
              # to the focused surface exactly as a physical keyboard's
              # would be. That is the distinction PLAT-38's gate rests
              # on: a synthesised call into `gpui_dispatch_event` would
              # test the binding against itself.
              #
              # `ydotool` is NOT here and is not an alternative: it
              # injects through `uinput`, which needs a privileged daemon
              # and a device node a CI container does not have — and a
              # key that never reached the compositor would be a
              # different experiment wearing the same name.
              pkgs.wtype
              # weston is deliberately NOT here. `weston
              # --backend=headless-backend.so` advertises no `wl_seat`
              # and GPUI unwraps that `None` at startup, so it cannot run
              # a GPUI client at all; `scripts/wayland-run-test.sh`
              # refuses `--compositor weston` with that reason. Shipping
              # it would only make the refusal look like a missing
              # package.
              pkgs.ffmpeg-full
              pkgs.mpv
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
              pkgs.mesa
              pkgs.libglvnd
              pkgs.stdenv.cc.cc.lib
            ]
          );

          shellHook = ''
            echo "isonim-gpui dev shell — rust $(rustc --version), nim $(nim --version 2>&1 | head -1)"
          '';
        };
      }
    );
}
