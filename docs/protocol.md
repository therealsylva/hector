# Deployed protocol notes

These notes describe the Nigerian web-client contract observed on 2026-09-01. They are implementation notes, not a stable public API guarantee.

## HTTP

The API root is `https://www.sportybet.com/api/ng/`. Hector currently exposes these read-only facts-center paths:

| Resource | Path |
| --- | --- |
| Sports | `factsCenter/sportList` |
| Live/prematch events | `factsCenter/liveOrPrematchEvents` |
| Event | `factsCenter/event` |
| Market groups | `factsCenter/marketGroups` |
| Outcomes | `factsCenter/Outcomes` |
| Balance/session check | `pocket/v1/finAccs/finAcc/userBal/{currency}` |

Requests use `OperId: 2` by default. Account calls also forward the imported `Cookie`, optional `DeviceId`, and optional `Fingerprint` headers. Hector surfaces CloudFront 403 responses and does not attempt to bypass WAF or CAPTCHA controls.

## Realtime registration and subscriptions

The default socket is:

```text
wss://alive-ng.sportybet.com/socket.io/?EIO=3&transport=websocket
```

After the Engine.IO open packet, Hector opens the Socket.IO namespace with `40`. When the server confirms with `40`, Hector emits a Socket.IO `data` event whose payload is:

```json
{
  "data": "{\"devType\":\"WEB\",\"deviceId\":\"...\",\"requestId\":1,\"productCode\":7}",
  "type": "reg"
}
```

Each subscription is another `data` event:

```json
{
  "data": "{\"topic\":\"...\",\"subType\":\"SUB\",\"pushType\":\"GROUP\",\"requestId\":2,\"productCode\":7}",
  "type": "sub"
}
```

Incoming `data` events use `type: "resp"` for request responses and `type: "ret"` for pushes. The nested `data.body` field is Base64-encoded UTF-8. Hector decodes it and parses JSON when possible. Engine.IO `2...` heartbeat packets are answered with matching `3...` packets.

Event topics have four caret-separated fields. Market topics have seven:

```text
sportId^categoryId^tournamentId^eventId^productId^marketId^marketSpecifiers
```

The web client uses `~` as its wildcard or empty marker.

## Transaction cipher

Protected write endpoints include `orders/order`, bet editing, cash-out, and auto-cash-out. Hector currently uses only `orders/order`.

1. Generate a random 16-byte AES key.
2. Base64-encode the key, URL-encode that string, and prepend `password=`.
3. Encrypt the resulting bytes with the deployed 1024-bit RSA public key using RSAES-PKCS1-v1_5.
4. Base64-encode the RSA ciphertext.
5. POST it as `text/plain` to `base/cipher`.
6. Retain the returned `data.transId` and AES key in memory. The web client treats the pair as valid for roughly one hour.

For a protected JSON request:

1. Serialize compact JSON.
2. Generate a fresh 16-byte IV.
3. Encrypt with AES-128-CBC and PKCS#7 padding.
4. Prepend the IV to the ciphertext and Base64-encode the combined bytes.
5. POST that string with `Content-Type: application/json;charset=UTF-8` and a `transId` header.

Protected responses use the same IV-prefix and AES-CBC framing. Key material is zeroized when Hector drops the transaction cipher.

## Single-order shape

The deployed single-selection payload is:

```json
{
  "bizType": 1,
  "ticket": {
    "selections": [
      {
        "eventId": "sr:match:123456",
        "id": "uof:3/sr:sport:1/1/1",
        "odds": "2.10",
        "banker": false,
        "probability": "0.48"
      }
    ],
    "bets": [
      {
        "selectedSystems": [1],
        "stake": { "value": 255000 }
      }
    ]
  },
  "orderType": 1,
  "paymentType": 0,
  "isBonusFactor": false,
  "subBizType": 1,
  "actualPayAmount": 255000
}
```

Money is scaled by 10,000. Product 1 maps to live `subBizType` 2; product 3 maps to prematch `subBizType` 1. A specifier is appended to the selection ID after `?`.

The deployed web client uses a 60-second pending timer and directs users to Bet History after a timeout. Hector follows the same operational invariant: it never retries order submission automatically and durably records a pending attempt before the POST.
