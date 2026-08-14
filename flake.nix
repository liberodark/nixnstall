{
  description = "nixnstall - declarative TUI installer for NixOS flake projects";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages = {
          nixnstall = pkgs.rustPlatform.buildRustPackage {
            pname = "nixnstall";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.makeWrapper ];
            postInstall = ''
              wrapProgram $out/bin/nixnstall \
                --prefix PATH : ${
                  pkgs.lib.makeBinPath [
                    pkgs.util-linux
                    pkgs.mkpasswd
                    pkgs.nixos-install-tools
                  ]
                }
            '';

            meta = {
              description = "Declarative TUI installer for NixOS flake projects";
              license = pkgs.lib.licenses.gpl3Only;
              mainProgram = "nixnstall";
            };
          };
          default = self.packages.${system}.nixnstall;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            util-linux
            mkpasswd
          ];
        };
      }
    );
}
