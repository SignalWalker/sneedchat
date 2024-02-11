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
        in {
          nativeBuildInputs = [
            pkgs.pkg-config
          ];
          buildInputs = [
            pkgs.sqlite
            pkgs.openssl
            pkgs.zlib
          ];
          packages = [
            # python
            # pkgs.tor
          ];
          env.PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig:${pkgs.sqlite.dev}/lib/pkgconfig";

          LD_LIBRARY_PATH = std.concatStringsSep ":" ["${pkgs.sqlite.out}/lib" "${pkgs.openssl.out}/lib"];
        });
      });
    };
}
