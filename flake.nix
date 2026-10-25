{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {nixpkgs, ...}:
    let 
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
    in 
    {
      devShells.x86_64-linux.default = pkgs.mkShell {
        packages = with pkgs;[
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer
          wayland-utils
          ffmpeg
          ffmpeg.dev
          pipewire.dev
          pkg-config
          rustPlatform.bindgenHook
          libva-utils
        ];
      };
    };
}
