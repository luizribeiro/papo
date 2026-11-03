/*
  Reusable HTTPS receiver for Web Push integration tests.

  Import this NixOS module and set services.webpush-receiver.enable plus
  certificateFile and keyFile. listenAddress and port select the HTTPS socket.
  Every POST is appended as one JSON object to
  /var/lib/webpush-receiver/requests.jsonl; read that file from the test driver
  after waiting for the expected line count. Each object has path, headers, and
  a base64-encoded body.
*/
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.webpush-receiver;
  receiver = pkgs.writeText "webpush-receiver.py" ''
    import argparse
    import base64
    import http.server
    import json
    import ssl
    import threading

    parser = argparse.ArgumentParser()
    parser.add_argument("--address", required=True)
    parser.add_argument("--port", required=True, type=int)
    parser.add_argument("--certificate", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--records", required=True)
    args = parser.parse_args()
    write_lock = threading.Lock()

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            record = {
                "path": self.path,
                "headers": {key.lower(): value for key, value in self.headers.items()},
                "body": base64.b64encode(self.rfile.read(length)).decode("ascii"),
            }
            with write_lock, open(args.records, "a", encoding="utf-8") as output:
                output.write(json.dumps(record, sort_keys=True) + "\n")
                output.flush()
            self.send_response(201)
            self.end_headers()

        def log_message(self, format, *values):
            print(format % values, flush=True)

    server = http.server.ThreadingHTTPServer((args.address, args.port), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(args.certificate, args.key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    server.serve_forever()
  '';
in
{
  options.services.webpush-receiver = {
    enable = lib.mkEnableOption "the Web Push test receiver";
    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "0.0.0.0";
      description = "Address on which the HTTPS receiver listens.";
    };
    port = lib.mkOption {
      type = lib.types.port;
      default = 8443;
      description = "Port on which the HTTPS receiver listens.";
    };
    certificateFile = lib.mkOption {
      type = lib.types.path;
      description = "PEM certificate presented by the HTTPS receiver.";
    };
    keyFile = lib.mkOption {
      type = lib.types.path;
      description = "PEM private key for the HTTPS receiver.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.webpush-receiver = {
      description = "Web Push test receiver";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      serviceConfig = {
        DynamicUser = true;
        StateDirectory = "webpush-receiver";
        ExecStart = lib.escapeShellArgs [
          (lib.getExe pkgs.python3)
          receiver
          "--address"
          cfg.listenAddress
          "--port"
          (toString cfg.port)
          "--certificate"
          cfg.certificateFile
          "--key"
          cfg.keyFile
          "--records"
          "/var/lib/webpush-receiver/requests.jsonl"
        ];
      };
    };
  };
}
