# papo needs SQLite-backed history from the version used in production testing.
{ ergochat, fetchFromGitHub }:

ergochat.overrideAttrs (
  finalAttrs: _: {
    version = "2.19.1";
    src = fetchFromGitHub {
      owner = "ergochat";
      repo = "ergo";
      rev = "v${finalAttrs.version}";
      hash = "sha256-yzGLOpECalSOv1zBpVkyDlHGaHSsQNsAoNa2jgLpsgM=";
    };
    tags = [ "sqlite" ];
  }
)
