# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
#
# Usage:
#   nix build            # builds the default package (ptouch-gui)
#   nix run .#ptouch     # runs the CLI
#   nix develop          # enters the dev shell
#   nix flake check      # runs fmt, clippy and test checks
{
  description = "Command-line and GUI tools for Brother P-Touch label printers";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    { self, nixpkgs, crane }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forEachSystem = nixpkgs.lib.genAttrs systems;

      perSystem =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          craneLib = crane.mkLib pkgs;
          inherit (pkgs) lib;

          src = self;

          version = (builtins.fromTOML (builtins.readFile (src + "/Cargo.toml"))).workspace.package.version;

          # crane's filterCargoSources keeps Rust sources, TOML files and
          # Cargo.lock, so it drops the assets ptouch-gui embeds with
          # include_bytes!; keep anything below an assets directory.
          cargoSrc = lib.cleanSourceWith {
            inherit src;
            filter = path: type: craneLib.filterCargoSources path type || lib.hasInfix "/assets/" path;
          };

          # libusb1-sys (vendored) probes libudev with pkg-config, so the build
          # needs udev plus pkg-config in nativeBuildInputs.
          buildDeps = [ pkgs.udev ];

          commonArgs = {
            src = cargoSrc;
            inherit version;
            pname = "ptouch-rs";
            strictDeps = true;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = buildDeps;
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          # Opened with dlopen at runtime, so the linker records no NEEDED entry
          # and nixpkgs adds no rpath for them; name them explicitly:
          #   glutin                  -> EGL/GL
          #   winit Wayland backend   -> libwayland-client, libwayland-egl
          #   winit keyboard          -> libxkbcommon, libxkbcommon-x11
          #   winit X11 backend       -> libX11, libX11-xcb, libXcursor,
          #                              libXi, libXinerama, libXrender
          #   x11rb (dl-libxcb)       -> libxcb
          #   rfd xdg-desktop-portal  -> libdbus-1
          guiRuntimeLibs = [
            pkgs.libglvnd
            pkgs.wayland
            pkgs.libxkbcommon
            pkgs.libx11
            pkgs.libxcursor
            pkgs.libxi
            pkgs.libxinerama
            pkgs.libxrender
            pkgs.libxcb
            pkgs.dbus
          ];

          installUdevRule = ''
            install -Dm644 ${src + "/data/udev/20-usb-ptouch-permissions.rules"} \
              $out/lib/udev/rules.d/20-usb-ptouch-permissions.rules
          '';

          metaCommon = {
            homepage = "https://github.com/vowstar/ptouch-rs";
            license = with lib.licenses; [
              mit
              gpl3Plus
            ];
            platforms = systems;
          };

          ptouch-cli = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "ptouch-cli";
              cargoBuildExtraArgs = "--package ptouch-cli";
              # Workspace tests run once in checks.test
              doCheck = false;
              postInstall = installUdevRule;
              meta = metaCommon // {
                description = "Command-line tool for Brother P-Touch label printers";
                mainProgram = "ptouch";
              };
            }
          );

          ptouch-gui = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "ptouch-gui";
              cargoBuildExtraArgs = "--package ptouch-gui";
              doCheck = false;
              postInstall = ''
                ${installUdevRule}
                install -Dm644 ${src + "/data/io.github.vowstar.ptouch-gui.desktop"} \
                  $out/share/applications/io.github.vowstar.ptouch-gui.desktop
                install -Dm644 ${src + "/data/io.github.vowstar.ptouch-gui.svg"} \
                  $out/share/icons/hicolor/scalable/apps/io.github.vowstar.ptouch-gui.svg
              '';
              postFixup = ''
                patchelf --add-rpath ${lib.makeLibraryPath guiRuntimeLibs} $out/bin/ptouch-gui
              '';
              meta = metaCommon // {
                description = "GUI application for Brother P-Touch label printers";
                mainProgram = "ptouch-gui";
              };
            }
          );
        in
        {
          packages = {
            inherit ptouch-cli ptouch-gui;
            default = ptouch-gui;
          };

          apps = {
            ptouch = {
              type = "app";
              program = lib.getExe ptouch-cli;
            };
            ptouch-gui = {
              type = "app";
              program = lib.getExe ptouch-gui;
            };
            default = {
              type = "app";
              program = lib.getExe ptouch-gui;
            };
          };

          checks = {
            inherit ptouch-cli ptouch-gui;
            # crane appends -fmt, -clippy and -test to the common pname
            fmt = craneLib.cargoFmt (
              commonArgs
              // {
                # cargo fmt --all -- --check, matching the project lint job
                cargoExtraArgs = "--all";
              }
            );
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--workspace -- -D warnings";
              }
            );
            test = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                # The sandbox has no fontconfig setup; fontdb reads
                # FONTCONFIG_FILE, so point it at a config with one font family
                # for the ptouch-render text tests
                FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; };
              }
            );
          };

          devShells.default = craneLib.devShell {
            # cargo, rustc, clippy and rustfmt are included by craneLib.devShell
            packages = [
              pkgs.pkg-config
              pkgs.rust-analyzer
            ];
            buildInputs = buildDeps ++ guiRuntimeLibs;
            # Lets `cargo run -p ptouch-gui` find the dlopened GUI libraries
            env.LD_LIBRARY_PATH = lib.makeLibraryPath guiRuntimeLibs;
          };

          formatter = pkgs.nixfmt;
        };
    in
    {
      packages = forEachSystem (system: (perSystem system).packages);
      apps = forEachSystem (system: (perSystem system).apps);
      checks = forEachSystem (system: (perSystem system).checks);
      devShells = forEachSystem (system: (perSystem system).devShells);
      formatter = forEachSystem (system: (perSystem system).formatter);
    };
}
