A self-signed certificate for `localhost` and its private key, used only by
the WebSocket transport's tests (`src/cdp/websocket.rs`) to stand in for a
`wss://` endpoint. Nothing trusts it outside those tests. Made with:

    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
      -keyout localhost.key -out localhost.crt -days 36500 -subj "/CN=localhost" \
      -addext "subjectAltName=DNS:localhost" -addext "basicConstraints=critical,CA:FALSE" \
      -addext "extendedKeyUsage=serverAuth"
