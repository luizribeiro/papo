{
  ergochat,
  pkgs,
  webpushTestClient,
}:
let
  receiverAddress = "203.0.113.2";
  receiverPort = 8443;
  pki = pkgs.runCommand "webpush-test-pki" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir -p $out
    openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
      -subj /CN=papo-webpush-test-ca \
      -keyout $out/ca.key -out $out/ca.crt
    openssl req -newkey rsa:2048 -nodes \
      -subj /CN=${receiverAddress} \
      -addext subjectAltName=IP:${receiverAddress} \
      -keyout $out/receiver.key -out $out/receiver.csr
    openssl x509 -req -days 1 -in $out/receiver.csr \
      -CA $out/ca.crt -CAkey $out/ca.key -CAcreateserial \
      -copy_extensions copy -out $out/receiver.crt
  '';
  vlan = address: {
    virtualisation.vlans = [ 1 ];
    networking.interfaces.eth1.ipv4.addresses = [
      {
        inherit address;
        prefixLength = 24;
      }
    ];
  };
in
pkgs.testers.runNixOSTest {
  name = "papo-webpush";
  extraPythonPackages = python: [ python.cryptography ];

  nodes = {
    ergo =
      { lib, ... }:
      lib.mkMerge [
        (vlan "203.0.113.1")
        {
          networking.firewall.allowedTCPPorts = [ 6667 ];
          security.pki.certificateFiles = [ "${pki}/ca.crt" ];
          services.ergochat = {
            enable = true;
            settings = {
              network.name = "papo-webpush";
              server = {
                casemapping = "ascii";
                name = "ergo.test";
                listeners.":6667" = { };
              };
              accounts.multiclient = {
                enabled = true;
                allowed-by-default = true;
                always-on = "opt-in";
                auto-away = "opt-out";
              };
              webpush = {
                enabled = true;
                timeout = "5s";
                delay = "0s";
                subscriber = "https://papo.example/";
                max-subscriptions = 2;
              };
            };
          };
          systemd.services.ergochat.serviceConfig.ExecStart =
            lib.mkForce "${lib.getExe ergochat} run --conf /etc/ergo.yaml";
        }
      ];

    receiver =
      { lib, ... }:
      {
        imports = [ ./webpush-receiver.nix ];
        config = lib.mkMerge [
          (vlan receiverAddress)
          {
            networking.firewall.allowedTCPPorts = [ receiverPort ];
            services.webpush-receiver = {
              enable = true;
              listenAddress = receiverAddress;
              port = receiverPort;
              certificateFile = "${pki}/receiver.crt";
              keyFile = "${pki}/receiver.key";
            };
          }
        ];
      };
  };

  testScript = ''
    client = "${webpushTestClient}/bin/papo-webpush-test-client"
    endpoint = "https://${receiverAddress}:${toString receiverPort}"
    ${builtins.readFile ./webpush-vm.py}
  '';
}
