{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell rec {
  name = "shell-rust";
  nativeBuildInputs = with pkgs; [ pkg-config cargo rustc clang ];
  buildInputs = with pkgs; [ pipewire alsa-lib ];
  LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath buildInputs;
}
