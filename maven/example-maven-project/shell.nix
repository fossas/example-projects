{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    buildInputs = [ pkgs.maven ];
    shellHook =
      ''
        echo ""
        echo To build: mvn package
        echo To execute: mvn compile exec:java -Dexec.mainClass=com.example.app.App
      '';
}