load(":base_image.bzl", _base = "coldeer_base_image")
load(":container.bzl", _container = "coldeer_container")
load(":package.bzl", _package = "coldeer_package", _nix = "coldeer_nix_dependency")
coldeer_package = _package
coldeer_container = _container
coldeer_base_image = _base
coldeer_nix_dependency = _nix
