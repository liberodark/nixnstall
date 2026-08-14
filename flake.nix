{
  description = "nixstall - declarative TUI installer for NixOS flake projects";

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
          nixstall = pkgs.rustPlatform.buildRustPackage {
            pname = "nixstall";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;

            # The installer shells out to these; keep them on PATH so it works
            # from a bare rescue environment.
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postInstall = ''
              wrapProgram $out/bin/nixstall \
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
              mainProgram = "nixstall";
            };
          };
          default = self.packages.${system}.nixstall;
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
