{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    buildInputs = [ pkgs.scala pkgs.sbt ];
    shellHook =
      ''
        echo ""
        echo To build: sbt compile
      '';
}