{
  stdenv,
  fetchurl,
  pkg-config,
  c-ares,
  jansson,
  libargon2,
  libsodium,
  openssl,
  pcre2,
}:

stdenv.mkDerivation (finalAttrs: {
  pname = "unrealircd";
  version = "6.2.7";

  src = fetchurl {
    url = "https://www.unrealircd.org/downloads/unrealircd-${finalAttrs.version}.tar.gz";
    hash = "sha256-4HU3X0aTCb8dVBNLyRTsUKSuOwnabS2ywR+fqI1kk3A=";
  };

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [
    c-ares
    jansson
    libargon2
    libsodium
    openssl
    pcre2
  ];

  configureFlags = [
    "--enable-dynamic-linking"
    "--enable-ssl=${openssl.dev}"
    "--with-bindir=${placeholder "out"}/bin"
    "--with-scriptdir=${placeholder "out"}/libexec/unrealircd-tools"
    "--with-confdir=${placeholder "out"}/share/unrealircd/conf"
    "--with-modulesdir=${placeholder "out"}/lib/unrealircd/modules"
    "--with-docdir=${placeholder "out"}/share/doc/unrealircd"
    "--with-builddir=${placeholder "out"}/share/unrealircd/build"
    "--with-logdir=."
    "--with-cachedir=."
    "--with-tmpdir=."
    "--with-datadir=."
    "--with-pidfile=./unrealircd.pid"
    "--with-controlfile=./unrealircd.ctl"
    "--with-privatelibdir=no"
    "--with-system-argon2"
  ];

  preInstall = ''
    mkdir -p "$out/libexec/unrealircd-tools"
  '';

  postInstall = ''
    rm "$out/libexec/unrealircd-tools/source"
  '';

  enableParallelBuilding = true;
})
