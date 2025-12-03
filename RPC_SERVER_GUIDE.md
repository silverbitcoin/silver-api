# SilverBitcoin RPC Server - Quick Start Guide

## Overview
Bu rehber, SilverBitcoin RPC sunucusunu derleyip çalıştırmayı adım adım anlatır.

## Gereksinimler

1. **Rust** (1.70+)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source $HOME/.cargo/env
   ```

2. **Git**
   ```bash
   # macOS
   brew install git
   
   # Linux
   sudo apt-get install git
   ```

3. **Build Tools**
   ```bash
   # macOS
   xcode-select --install
   
   # Linux (Ubuntu/Debian)
   sudo apt-get install build-essential pkg-config libssl-dev
   ```

## Adım 1: Repository'yi Clone Edin

```bash
git clone https://github.com/SilverBitcoin/silverbitcoin.git
cd silverbitcoin-blockchain
```

## Adım 2: RPC Sunucusunu Derleyin

```bash
# Debug mode (hızlı derleme, yavaş çalışma)
cargo build -p silver-api

# Release mode (yavaş derleme, hızlı çalışma) - ÖNERİLİ
cargo build -p silver-api --release
```

Derleme 5-15 dakika sürebilir (ilk kez daha uzun olabilir).

## Adım 3: RPC Sunucusunu Çalıştırın

### Basit Başlangıç (Varsayılan Ayarlar)

```bash
# Debug mode
./target/debug/silver-rpc-server

# Release mode (ÖNERİLİ)
./target/release/silver-rpc-server
```

Çıktı şöyle görünmelidir:
```
2025-01-19T10:30:45.123Z  INFO silver_api: Starting SilverBitcoin RPC Server
2025-01-19T10:30:45.124Z  INFO silver_api: HTTP: 127.0.0.1:9000
2025-01-19T10:30:45.125Z  INFO silver_api: WebSocket: 127.0.0.1:9001
2025-01-19T10:30:45.126Z  INFO silver_api: Database: ./data
2025-01-19T10:30:45.500Z  INFO silver_api: Database initialized successfully
2025-01-19T10:30:45.501Z  INFO silver_api: Starting RPC servers...
2025-01-19T10:30:45.502Z  INFO silver_api: ✓ RPC Server started successfully
2025-01-19T10:30:45.503Z  INFO silver_api: HTTP endpoint: http://127.0.0.1:9000
2025-01-19T10:30:45.504Z  INFO silver_api: WebSocket endpoint: ws://127.0.0.1:9001
2025-01-19T10:30:45.505Z  INFO silver_api: Press Ctrl+C to stop
```

### Özel Ayarlarla Başlangıç

```bash
# Özel port ve database dizini
./target/release/silver-rpc-server \
  --http 0.0.0.0:9000 \
  --ws 0.0.0.0:9001 \
  --db /var/lib/silverbitcoin/data \
  --max-connections 5000 \
  --rate-limit 200 \
  --log-level debug

# Tüm seçenekler
./target/release/silver-rpc-server --help
```

## Adım 4: RPC Sunucusunu Test Edin

### cURL ile Test

```bash
# Basit test
curl -X POST http://127.0.0.1:9000 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "silver_getLatestBlockNumber",
    "params": [],
    "id": 1
  }'

# Beklenen yanıt:
# {"jsonrpc":"2.0","result":{"block_number":12345},"id":1}
```

### Node.js ile Test

```bash
# npm install axios
node -e "
const axios = require('axios');
axios.post('http://127.0.0.1:9000', {
  jsonrpc: '2.0',
  method: 'silver_getLatestBlockNumber',
  params: [],
  id: 1
}).then(r => console.log(JSON.stringify(r.data, null, 2)))
  .catch(e => console.error(e.message));
"
```

### Python ile Test

```bash
# pip install requests
python3 -c "
import requests
import json

response = requests.post('http://127.0.0.1:9000', json={
    'jsonrpc': '2.0',
    'method': 'silver_getLatestBlockNumber',
    'params': [],
    'id': 1
})
print(json.dumps(response.json(), indent=2))
"
```

## Adım 5: Explorer'ı Bağlayın

Explorer'ın `.env.local` dosyasını güncelleyin:

```env
NEXT_PUBLIC_RPC_URL=http://127.0.0.1:9000
NEXT_PUBLIC_CHAIN_ID=5200
NEXT_PUBLIC_CHAIN_NAME=SilverBitcoin Mainnet
NEXT_PUBLIC_CURRENCY_SYMBOL=SBTC
NEXT_PUBLIC_CURRENCY_DECIMALS=9
```

Sonra explorer'ı başlatın:

```bash
cd silverbitcoin-explorer
npm run dev
```

Explorer şu adreste açılacak: http://localhost:3000

## Komut Satırı Seçenekleri

```
--http <ADDR>              HTTP sunucusu bind adresi (default: 127.0.0.1:9000)
--ws <ADDR>                WebSocket sunucusu bind adresi (default: 127.0.0.1:9001)
--db <PATH>                Database dizini (default: ./data)
--max-connections <NUM>    Maksimum bağlantı sayısı (default: 1000)
--rate-limit <NUM>         IP başına istek/saniye (default: 100)
--enable-cors <BOOL>       CORS etkinleştir (default: true)
--log-level <LEVEL>        Log seviyesi: trace, debug, info, warn, error (default: info)
--help                     Yardım göster
```

## Sorun Giderme

### Port Zaten Kullanımda

```bash
# Port 9000'ı kullanan işlemi bul
lsof -i :9000

# Farklı port kullan
./target/release/silver-rpc-server --http 127.0.0.1:9002
```

### Database Hataları

```bash
# Database'i sıfırla
rm -rf ./data

# Yeniden başlat
./target/release/silver-rpc-server
```

### Derleme Hataları

```bash
# Cargo cache'i temizle
cargo clean

# Yeniden derle
cargo build -p silver-api --release
```

### Bağlantı Reddedildi

```bash
# Sunucunun çalışıp çalışmadığını kontrol et
curl http://127.0.0.1:9000

# Firewall ayarlarını kontrol et
sudo ufw allow 9000/tcp
sudo ufw allow 9001/tcp
```

## Docker ile Çalıştırma

```dockerfile
# Dockerfile
FROM rust:1.70 as builder
WORKDIR /app
COPY . .
RUN cargo build -p silver-api --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates
COPY --from=builder /app/target/release/silver-rpc-server /usr/local/bin/
EXPOSE 9000 9001
CMD ["silver-rpc-server", "--http", "0.0.0.0:9000", "--ws", "0.0.0.0:9001"]
```

```bash
# Build
docker build -t silverbitcoin-rpc .

# Run
docker run -p 9000:9000 -p 9001:9001 -v rpc-data:/data silverbitcoin-rpc
```

## Üretim Ortamı Ayarları

```bash
# Systemd service dosyası (/etc/systemd/system/silverbitcoin-rpc.service)
[Unit]
Description=SilverBitcoin RPC Server
After=network.target

[Service]
Type=simple
User=silverbitcoin
WorkingDirectory=/opt/silverbitcoin
ExecStart=/opt/silverbitcoin/silver-rpc-server \
  --http 0.0.0.0:9000 \
  --ws 0.0.0.0:9001 \
  --db /var/lib/silverbitcoin/data \
  --max-connections 5000 \
  --rate-limit 200 \
  --log-level info
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

```bash
# Etkinleştir ve başlat
sudo systemctl enable silverbitcoin-rpc
sudo systemctl start silverbitcoin-rpc

# Durumu kontrol et
sudo systemctl status silverbitcoin-rpc

# Logları görüntüle
sudo journalctl -u silverbitcoin-rpc -f
```

## Nginx Reverse Proxy (Üretim)

```nginx
upstream silverbitcoin_rpc {
    server 127.0.0.1:9000;
}

server {
    listen 80;
    server_name rpc.silverbitcoin.org;

    location / {
        proxy_pass http://silverbitcoin_rpc;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        
        # WebSocket support
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

## Performans İpuçları

1. **Release Mode Kullan**: Debug mode'dan 10x daha hızlı
2. **SSD Kullan**: Database performansı için
3. **Yeterli RAM**: En az 4GB önerilir
4. **Rate Limiting**: DDoS koruması için etkinleştir
5. **Monitoring**: Prometheus metrikleri ekle

## Destek

- Dokumentasyon: https://silverbitcoin.org/docs
- GitHub Issues: https://github.com/SilverBitcoin/silverbitcoin-blockchain/issues
- Discord: https://discord.com/invite/MCGn7dzvgd
