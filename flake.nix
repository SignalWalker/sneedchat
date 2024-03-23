{
  description = "";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    alejandra = {
      url = "github:kamadorueda/alejandra";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs = inputs @ {
    self,
    nixpkgs,
    ...
  }:
    with builtins; let
      std = nixpkgs.lib;
      systems = ["x86_64-linux"];
      nixpkgsFor = std.genAttrs systems (system:
        import nixpkgs {
          localSystem = builtins.currentSystem or system;
          crossSystem = system;
          overlays = [];
        });
    in {
      formatter = std.mapAttrs (system: pkgs: pkgs.default) inputs.alejandra.packages;
      devShells = std.genAttrs systems (system: let
        pkgs = nixpkgsFor.${system};
      in {
        default = pkgs.mkShell (let
          python = pkgs.python311.withPackages (ps:
            with ps; [
              cryptography
              stem
            ]);
          libraries = with pkgs; [
            sqlite
            openssl
            zlib
            xz
            # tauri
            webkitgtk_4_1 # dioxus requires javascriptcoregtk-4-1
            gtk3
            gdk-pixbuf
            glib
            dbus
            librsvg
            # dioxus
            libsoup_3
            xdotool
            pango
            cairo
            ## dioxus-cli
            bzip2
          ];
          buildInputs = with pkgs; [
            sqlite
            openssl
            zlib
            # tauri
            curl
            wget
            dbus
            glib
            gtk3
            libsoup
            webkitgtk_4_1 # dioxus requires javascriptcoregtk-4-1
            librsvg
            # dioxus
            xdotool
            pango
          ];
        in {
          nativeBuildInputs = [
            pkgs.pkg-config
          ];
          buildInputs = buildInputs;
          packages = with pkgs; [
            nasm # for building dioxus-cli
            tailwindcss
          ];

          shellHook = let
            extraLdPaths = pkgs.lib.makeLibraryPath libraries;
            extraDataDirs = std.concatStringsSep ":" [
              "${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}"
              "${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}"
            ];
          in ''
            export LD_LIBRARY_PATH="${extraLdPaths}:$LD_LIBRARY_PATH"
            export XDG_DATA_DIRS="${extraDataDirs}:$XDG_DATA_DIRS"
            export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules"
          '';
        });
      });
    };
}
