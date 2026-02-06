# Product Requirements Document: BambuPrint IQ

**Version:** 1.1
**Author:** Michael Curtis & Claude
**Date:** February 4, 2026
**Status:** Draft

---

## Executive Summary

**BambuPrint IQ** is an intelligent companion app for Bambu Lab 3D printers that bridges the gap between novice frustration and expert-level print quality. By combining AI-powered print analysis, automated profile generation, real-time monitoring, and environmental awareness, we're creating the "missing piece" of the Bambu ecosystem—a tool that learns from your prints and continuously optimizes your settings.

### The Problem

Despite Bambu Lab's excellent hardware, users face significant friction:

1. **Profile Paralysis**: New users struggle to dial in settings for third-party filaments
2. **Reactive Troubleshooting**: Print failures are discovered after hours of wasted time and material
3. **Environmental Blindspots**: Humidity and temperature affect prints, but data isn't actionable
4. **Fragmented Ecosystem**: Profile management across multiple printers/filaments is cumbersome
5. **Third-Party Restrictions**: Recent firmware changes (Jan 2025) have limited community tools, creating demand for smart solutions that work within Bambu's guidelines

### The Solution

An intelligent platform that:
- **Sees** your prints (AI analysis of test prints and in-progress photos)
- **Learns** optimal settings (web-scraped filament data + community profiles)
- **Monitors** your environment (MQTT telemetry + sensor integration)
- **Optimizes** automatically (generates and imports Bambu Studio profiles)

---

## Market Context & Research Findings

### Bambu Lab Ecosystem (2025)

| Component | Access Method | Data Available |
|-----------|--------------|----------------|
| **Printer Telemetry** | MQTT (port 8883) | Temps, speeds, filament usage, AMS data |
| **Camera Feeds** | RTSP (X1) / JPEG frames (A1/P1) | 1 FPS on P1/A1, higher on X1 |
| **AMS Humidity** | MQTT | 5 levels (A-E) based on water content |
| **Profile Format** | JSON | Inheritable presets with filament/process separation |
| **File Upload** | FTP / Cloud API | Direct print job submission |

### Available Python Libraries

- `bambu-lab-cloud-api` - Cloud API, MQTT, FTP, video streams
- `bambulabs-api` - Programmatic printer control
- Third-party: Home Assistant integration, Prometheus exporters

### AI/ML State of the Art

- **Defect Detection**: CNNs achieve 98-99% accuracy on print defects
- **Real-time Analysis**: YOLOv8 enables live monitoring
- **Bambu Built-in**: X1 has spaghetti detection via NPU
- **Gap**: No comprehensive quality analysis → settings recommendation pipeline

---

## January 2025 Authorization System & Bambu Connect

### Background

On January 16, 2025, Bambu Lab announced firmware changes introducing an authorization control system. This was in response to security concerns (reportedly 30 million unauthorized API requests per day and DDoS attacks). The changes significantly impact how third-party applications interact with Bambu printers.

### What Changed

| Operation | Pre-Jan 2025 | Post-Jan 2025 |
|-----------|--------------|---------------|
| **MQTT Status Push** (telemetry) | ✅ Open | ✅ Still Open |
| **Start Print Jobs** | ✅ Open | 🔐 Requires Auth |
| **Camera/Video Stream** | ✅ Open | 🔐 Requires Auth |
| **Control Motion/Temps/Fans** | ✅ Open | 🔐 Requires Auth |
| **Firmware Updates** | ✅ Open | 🔐 Requires Auth |
| **FTP File Upload** | ✅ Open | 🔐 Requires Auth |
| **SD Card Printing** | ✅ Open | ✅ Still Open |

### Integration Options (Post-Jan 2025)

#### Option 1: Bambu Connect Integration (Recommended)

**What it is**: Official middleware application that handles authentication between third-party apps and Bambu printers.

**How it works**:
1. User installs Bambu Connect on their computer
2. Bambu Connect authenticates with Bambu Cloud
3. Third-party apps communicate through Bambu Connect
4. All commands pass through secure, verified channels

**SDK Access Process**:
1. Create Bambu Lab account at bambulab.com
2. Submit SDK access request via official form
3. Provide: application name, use cases, functionality description
4. Bambu Lab reviews and grants access
5. Receive Local Server SDK binary for integration

**Integration Methods**:
- **Package SDK Binary**: Embed Local Server SDK within our application
- **Communicate with Bambu Connect**: If user has it installed, connect to its local API
- **Cloud API with 2FA**: Direct cloud authentication with email verification

#### Option 2: Developer Mode

**What it is**: Manual setting on printer that opens MQTT, video stream, and FTP without authorization.

**Trade-offs**:
- ✅ Full local control
- ❌ Loses cloud features (remote access, notifications)
- ❌ User assumes security responsibility
- ❌ No official Bambu Lab support

**Best for**: Power users, print farms with isolated networks, privacy-focused users

#### Option 3: Cloud API Authentication

**What it is**: Direct authentication with Bambu Cloud using email + verification code.

**Authentication Flow**:
```
1. App requests login with user email
2. Bambu sends verification code to email
3. User enters code in app
4. App receives auth token (saved to ~/.bambu_token)
5. Token auto-refreshes as needed
```

**Capabilities via Cloud API**:
- Device listing and management
- Real-time MQTT (authenticated)
- Video streaming (TTCode credentials)
- File upload via S3 signed URLs
- Full printer control

### Our Integration Strategy

**Primary Path: Bambu Connect + Cloud API**

We will pursue official SDK access and support multiple auth methods:

```
┌─────────────────────────────────────────────────────────────────┐
│                    BambuPrint IQ                                │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              Authentication Manager                      │   │
│  │  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐       │   │
│  │  │ Bambu Cloud │ │   Bambu     │ │  Developer  │       │   │
│  │  │  API Auth   │ │  Connect    │ │    Mode     │       │   │
│  │  │  (2FA)      │ │  (SDK)      │ │  (Direct)   │       │   │
│  │  └──────┬──────┘ └──────┬──────┘ └──────┬──────┘       │   │
│  │         │               │               │               │   │
│  │         └───────────────┼───────────────┘               │   │
│  │                         ▼                               │   │
│  │              Unified Printer Interface                  │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

**Feature Availability by Auth Method**:

| Feature | No Auth | Cloud API | Bambu Connect | Dev Mode |
|---------|---------|-----------|---------------|----------|
| Read Telemetry | ✅ | ✅ | ✅ | ✅ |
| AMS Humidity | ✅ | ✅ | ✅ | ✅ |
| Camera Stream | ❌ | ✅ | ✅ | ✅ |
| Start Print | ❌ | ✅ | ✅ | ✅ |
| Control Printer | ❌ | ✅ | ✅ | ✅ |
| Works Offline | ✅ | ❌ | ✅ | ✅ |
| Cloud Features | N/A | ✅ | ✅ | ❌ |

### SDK Application Plan

**Timeline**: Apply during Phase 1 development

**Application Contents**:
- Company/Developer: Michael Curtis
- Application: BambuPrint IQ
- Use Cases: Print quality analysis, profile optimization, monitoring
- Functionality: Read telemetry, capture camera frames for AI analysis, profile management
- Distribution: Desktop app (Electron/Tauri) + Web dashboard

**Fallback if SDK Denied**:
- Cloud API with user-authenticated tokens
- Developer Mode support for power users
- Core MVP (profile generation) works without any printer connection

---

## Product Vision

### Target Users

| Persona | Pain Points | Value Proposition |
|---------|-------------|-------------------|
| **Hobbyist Hannah** | New to 3D printing, overwhelmed by settings | "Upload a Benchy photo, get perfect settings" |
| **Prosumer Pete** | Uses many filament brands, hates manual tuning | "Auto-generate profiles from filament specs" |
| **Farm Manager Fiona** | Runs 10+ printers, needs centralized control | "Dashboard showing all printers, predictive alerts" |
| **Tinkerer Tom** | Wants data, loves optimization | "Deep analytics, A/B testing profiles" |

### Core Value Loop

```
Print Test → Capture Photo → AI Analysis → Web Scrape Specs →
Generate Profile → Import to Bambu Studio → Print Again → Measure Improvement
```

---

## Feature Specifications

### Phase 1: Smart Profile Generator (MVP)

#### F1.1: Benchy Analysis Engine

**Description**: Upload a photo of a 3D Benchy (or other calibration print), receive AI-powered diagnosis of print quality issues.

**User Flow**:
1. User uploads photo(s) of completed print
2. AI analyzes for common defects:
   - Layer adhesion issues
   - Stringing/oozing
   - Overhangs drooping
   - Surface roughness
   - Z-banding
   - Elephant's foot
   - Warping
3. System returns severity scores and recommended setting changes

**Technical Approach**:
- Vision model (Claude/GPT-4V or fine-tuned CNN)
- Defect classification with confidence scores
- Mapping defects → setting adjustments (rule engine + ML)

**Output Format**:
```json
{
  "analysis": {
    "overall_score": 7.2,
    "defects": [
      {
        "type": "stringing",
        "severity": "moderate",
        "confidence": 0.89,
        "recommendations": [
          {"setting": "retraction_length", "current": 0.8, "suggested": 1.2},
          {"setting": "nozzle_temperature", "adjustment": -5}
        ]
      }
    ]
  }
}
```

#### F1.2: Filament Spec Scraper

**Description**: Given a filament brand/name, automatically fetch recommended settings from manufacturer websites, datasheets, and community sources.

**Data Sources** (priority order):
1. Manufacturer product pages
2. Technical datasheets (PDF parsing)
3. Community databases (e.g., MakerWorld profiles)
4. User-contributed settings

**Scraped Parameters**:
- Print temperature range (nozzle)
- Bed temperature range
- Print speed recommendations
- Cooling requirements
- Drying recommendations
- Density/diameter specs
- Special notes (enclosure required, etc.)

**Technical Approach**:
- Web scraping with intelligent parsing
- LLM extraction from unstructured pages
- Caching layer for performance
- User corrections fed back to improve accuracy

#### F1.3: Profile Generator

**Description**: Combine analysis results + scraped specs + printer model to generate optimized Bambu Studio JSON profiles.

**Profile Structure** (matching Bambu format):
```json
{
  "filament_id": "GFU99_CUSTOM_001",
  "setting_id": "GFS99_CUSTOM_001",
  "name": "Polymaker PLA Pro @0.4 nozzle",
  "inherits": "Generic PLA",
  "instantiation": true,
  "compatible_printers": ["Bambu Lab P1S 0.4 nozzle"],
  "nozzle_temperature": [215],
  "bed_temperature": [60],
  "fan_speed": [80],
  "retraction_length": [1.2],
  // ... full parameter set
}
```

**Auto-Import**:
- Generate .json file
- Option 1: User manually imports via Bambu Studio
- Option 2: Direct file placement in AppData folder (with user permission)
- Option 3: API integration if Bambu opens this up

---

### Phase 2: Real-Time Print Monitor

#### F2.1: MQTT Telemetry Dashboard

**Description**: Connect to printer(s) via MQTT and display real-time metrics.

**Connection Flow**:
1. User provides printer IP + LAN password (from printer display)
2. App connects to `<ip>:8883` with TLS
3. Subscribe to `device/<serial>/report`
4. Parse and display telemetry

**Displayed Metrics**:
- Nozzle temperature (actual vs target)
- Bed temperature (actual vs target)
- Print progress (%)
- Current layer / total layers
- Estimated time remaining
- AMS humidity levels (A-E per slot)
- Filament usage
- Print speed (actual)

**Alerts**:
- Temperature deviation > threshold
- AMS humidity warning (level D or E)
- Print stalled detection
- Filament runout warning

#### F2.2: Camera Integration

**Description**: Pull camera frames for monitoring and AI analysis.

**X1 Series**:
- RTSP stream: `rtsps://<ip>/streaming/live/1`
- Higher frame rate available

**P1/A1 Series**:
- JPEG frame extraction (1 FPS limitation)
- Via MQTT or direct endpoint

**Features**:
- Live view in dashboard
- Timelapse generation
- Periodic AI analysis for defect detection during print
- "Spaghetti alert" backup (complement Bambu's built-in)

#### F2.3: Environmental Monitoring

**Description**: Track and correlate environmental conditions with print quality.

**Data Sources**:
- AMS built-in humidity (via MQTT)
- Optional: BIGTREETECH Panda Sense integration
- Optional: Generic BLE sensors (Xiaomi/Aqara via bridge)
- Room temperature/humidity

**Insights**:
- "Your PLA prints fail 3x more often when humidity > 60%"
- "This filament performed best at 23°C ambient"
- Drying reminders based on filament type + exposure time

---

### Phase 3: Print Farm & Analytics

#### F3.1: Multi-Printer Dashboard

**Description**: Centralized view of all connected Bambu printers.

**Features**:
- Grid view with printer status cards
- Real-time status: Idle / Printing / Error / Offline
- Queue management across printers
- Batch profile deployment

#### F3.2: Print History & Analytics

**Description**: Track every print with outcomes and correlate to settings.

**Tracked Data**:
- Print file / model name
- Filament used (brand, type, color)
- Profile settings snapshot
- Duration (estimated vs actual)
- Environmental conditions during print
- Outcome (success / fail / partial)
- User quality rating (1-5 stars)
- Photos (before/during/after)

**Analytics**:
- Success rate by filament type
- Average quality score over time
- Setting correlations ("higher retraction = less stringing")
- Predictive: "This print has 73% chance of stringing based on similar jobs"

#### F3.3: Community Profile Exchange

**Description**: Share and discover profiles from other users.

**Features**:
- Upload profiles with metadata (printer, filament, conditions)
- Browse/search community profiles
- Rating and verification system
- "Works for me" confirmations
- Privacy controls (anonymous sharing option)

---

## Technical Architecture

### Recommended Stack (per user preferences)

```
┌─────────────────────────────────────────────────────────────────┐
│                        FRONTEND                                 │
│                   Next.js / React                               │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐       │
│  │ Dashboard│  │ Analysis │  │ Profiles │  │ Settings │       │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘       │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        BACKEND                                  │
│              Python (FastAPI) or Rust (Axum)                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                    API Gateway                            │  │
│  └──────────────────────────────────────────────────────────┘  │
│       │              │              │              │            │
│       ▼              ▼              ▼              ▼            │
│  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────────┐    │
│  │ AI/ML   │   │ Scraper │   │ Profile │   │   Bambu     │    │
│  │ Service │   │ Service │   │ Gen Svc │   │ Auth Bridge │    │
│  └─────────┘   └─────────┘   └─────────┘   └─────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
         ┌────────────────────┼────────────────────┐
         ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│  BAMBU CONNECT  │  │   CLOUD API     │  │  DEVELOPER MODE │
│  (Local SDK)    │  │   (2FA Auth)    │  │  (Direct MQTT)  │
│                 │  │                 │  │                 │
│ • Full control  │  │ • Full control  │  │ • Full control  │
│ • Works offline │  │ • Cloud features│  │ • No cloud      │
│ • SDK required  │  │ • Needs internet│  │ • User enables  │
└────────┬────────┘  └────────┬────────┘  └────────┬────────┘
         │                    │                    │
         └────────────────────┼────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    BAMBU PRINTERS                               │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                Unified Printer Interface                  │  │
│  │  • MQTT Telemetry (temps, progress, AMS)                 │  │
│  │  • Camera Streams (RTSP/JPEG)                            │  │
│  │  • Print Control (start, pause, stop)                    │  │
│  │  • File Upload (FTP/Cloud)                               │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      DATA LAYER                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐       │
│  │ PostgreSQL│ │  Redis   │  │ S3/Blob  │  │ Vector DB │       │
│  │ (profiles)│ │ (cache)  │  │ (images) │  │(embeddings)│      │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘       │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    EXTERNAL SYSTEMS                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐       │
│  │ Bambu    │  │ Filament │  │ AI Vision│  │ Community │       │
│  │ Cloud    │  │ Websites │  │  APIs    │  │   APIs    │       │
│  │ (Auth)   │  │ (Scrape) │  │(Claude/  │  │           │       │
│  │          │  │          │  │ OpenAI)  │  │           │       │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘       │
└─────────────────────────────────────────────────────────────────┘
```

### Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **Frontend** | Next.js 14+ | SSR for dashboard, React ecosystem, your preference |
| **Backend Option A** | Python (FastAPI) | Rich ML libraries, existing Bambu packages, fast development |
| **Backend Option B** | Rust (Axum) | Performance for MQTT handling, memory safety, your preference |
| **Hybrid Approach** | Python ML services + Rust core | Best of both worlds |
| **Database** | PostgreSQL | Relational data (profiles, history), JSON support |
| **Cache** | Redis | MQTT message buffering, session state |
| **Image Storage** | S3-compatible | Print photos, timelapse frames |
| **AI Vision** | Claude API / GPT-4V | Multimodal analysis without training custom models (initially) |
| **MQTT Client** | `paho-mqtt` (Python) or `rumqttc` (Rust) | Mature libraries |

### Bambu Auth Bridge Service

The Auth Bridge abstracts authentication complexity, supporting all three connection methods:

```python
# bambu_auth_bridge.py - Unified authentication interface

from enum import Enum
from dataclasses import dataclass
from typing import Optional
import asyncio

class AuthMethod(Enum):
    CLOUD_API = "cloud"      # 2FA with email verification
    BAMBU_CONNECT = "sdk"    # Local SDK integration
    DEVELOPER_MODE = "dev"   # Direct MQTT (no auth)

@dataclass
class PrinterConnection:
    serial: str
    ip: Optional[str]
    auth_method: AuthMethod
    token: Optional[str]

class BambuAuthBridge:
    """Unified interface for all Bambu auth methods"""

    async def connect_cloud(self, email: str) -> str:
        """
        Cloud API authentication flow:
        1. Request verification code sent to email
        2. User enters code
        3. Receive and cache auth token
        """
        # Request code
        await self._request_verification_code(email)
        # Token saved to ~/.bambu_token
        return "awaiting_verification"

    async def verify_code(self, email: str, code: str) -> PrinterConnection:
        """Complete 2FA verification"""
        token = await self._verify_and_get_token(email, code)
        return PrinterConnection(
            serial=await self._get_device_serial(token),
            ip=None,  # Cloud mode
            auth_method=AuthMethod.CLOUD_API,
            token=token
        )

    async def connect_bambu_connect(self, printer_ip: str) -> PrinterConnection:
        """
        Connect via Bambu Connect SDK:
        - Requires SDK binary bundled or Bambu Connect installed
        - Communicates via local socket
        """
        serial = await self._discover_via_sdk(printer_ip)
        return PrinterConnection(
            serial=serial,
            ip=printer_ip,
            auth_method=AuthMethod.BAMBU_CONNECT,
            token=None  # SDK handles auth
        )

    async def connect_developer_mode(
        self, printer_ip: str, lan_password: str
    ) -> PrinterConnection:
        """
        Direct MQTT connection (requires Developer Mode enabled on printer)
        """
        return PrinterConnection(
            serial=await self._get_serial_direct(printer_ip, lan_password),
            ip=printer_ip,
            auth_method=AuthMethod.DEVELOPER_MODE,
            token=lan_password
        )
```

### MQTT Telemetry (Works Without Auth)

Status push is still available without authentication on firmware 01.08+:

```python
# telemetry_client.py - Read-only monitoring (no auth required)

import paho.mqtt.client as mqtt
import ssl
import json

class TelemetryClient:
    """Read-only MQTT client for printer telemetry"""

    def __init__(self, printer_ip: str, serial: str, lan_password: str):
        self.client = mqtt.Client()
        self.client.username_pw_set("bblp", lan_password)
        self.client.tls_set(cert_reqs=ssl.CERT_NONE)
        self.client.on_message = self._on_message
        self.printer_ip = printer_ip
        self.serial = serial

    def connect(self):
        self.client.connect(self.printer_ip, 8883)
        self.client.subscribe(f"device/{self.serial}/report")
        self.client.loop_start()

    def _on_message(self, client, userdata, msg):
        data = json.loads(msg.payload)
        # Available without auth:
        # - temperatures (nozzle, bed, chamber)
        # - print progress
        # - AMS humidity
        # - fan speeds
        # - error codes (HMS)
        self.on_telemetry(data)
```

**Telemetry Message Format** (available without auth):
```json
{
  "print": {
    "gcode_state": "RUNNING",
    "mc_percent": 45,
    "mc_remaining_time": 3600,
    "layer_num": 42,
    "total_layer_num": 200
  },
  "nozzle_temper": 215.0,
  "nozzle_target_temper": 215.0,
  "bed_temper": 60.0,
  "bed_target_temper": 60.0,
  "chamber_temper": 35.0,
  "fan_gear": 15,
  "heatbreak_fan_speed": "7",
  "cooling_fan_speed": "15",
  "wifi_signal": "-52dBm",
  "ams": {
    "humidity": "B",
    "tray": [
      {"id": 0, "tray_type": "PLA", "remain": 85, "k": 0.02}
    ]
  },
  "hms": []  // Error codes
}
```

### Authenticated Operations

For camera, print control, and file upload, use the Auth Bridge:

```python
# printer_control.py - Authenticated operations

class PrinterControl:
    """Full printer control (requires authentication)"""

    def __init__(self, connection: PrinterConnection):
        self.conn = connection

    async def get_camera_stream(self) -> str:
        """
        Returns stream URL based on auth method:
        - Cloud API: TTCode credentials for P2P
        - Bambu Connect: Local RTSP URL
        - Developer Mode: Direct RTSP
        """
        if self.conn.auth_method == AuthMethod.CLOUD_API:
            ttcode = await self._get_ttcode()
            return f"tutk://{ttcode}"
        else:
            return f"rtsps://{self.conn.ip}/streaming/live/1"

    async def start_print(self, file_path: str):
        """Start a print job (requires auth)"""
        if self.conn.auth_method == AuthMethod.CLOUD_API:
            await self._upload_to_cloud(file_path)
        else:
            await self._upload_via_ftp(file_path)
        await self._send_print_command()

    async def capture_frame(self) -> bytes:
        """Capture single frame for AI analysis"""
        # X1: Extract from RTSP
        # P1/A1: Fetch JPEG frame
        pass
```

---

## User Experience

### Onboarding Flow

```
┌─────────────────────────────────────────────────────────────────┐
│ 1. WELCOME                                                      │
│    "BambuPrint IQ makes every print better"                     │
│    [Get Started]                                                │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ 2. CHOOSE CONNECTION METHOD                                     │
│                                                                 │
│    ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│    │  ☁️ Cloud   │  │ 🔌 Local    │  │ 🔧 Advanced │          │
│    │   Login     │  │  (Bambu     │  │ (Developer  │          │
│    │             │  │  Connect)   │  │    Mode)    │          │
│    │ Recommended │  │             │  │             │          │
│    └─────────────┘  └─────────────┘  └─────────────┘          │
│                                                                 │
│    "Don't have a printer yet? [Skip to Profile Generator]"      │
└─────────────────────────────────────────────────────────────────┘
                              │
            ┌─────────────────┼─────────────────┐
            ▼                 ▼                 ▼
┌───────────────────┐ ┌───────────────┐ ┌───────────────────┐
│ 3a. CLOUD LOGIN   │ │ 3b. LOCAL     │ │ 3c. DEV MODE      │
│                   │ │               │ │                   │
│ Email: [       ]  │ │ Printer IP:   │ │ Printer IP:       │
│ [Send Code]       │ │ [192.168.x.x] │ │ [192.168.x.x]     │
│                   │ │               │ │                   │
│ Code: [      ]    │ │ [Detect via   │ │ LAN Password:     │
│ [Verify]          │ │  Bambu        │ │ [            ]    │
│                   │ │  Connect]     │ │                   │
│ ✅ Token saved    │ │               │ │ ⚠️ Requires Dev   │
│                   │ │ ✅ Connected  │ │   Mode enabled    │
└───────────────────┘ └───────────────┘ └───────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ 4. PRINTER DETECTED                                             │
│                                                                 │
│    🖨️ Bambu Lab P1S                                            │
│    Serial: 01P00A123456789                                      │
│    Firmware: 01.08.03.00                                        │
│    Status: Idle                                                 │
│    AMS: 4 slots (Humidity: B)                                   │
│                                                                 │
│    [Continue to Dashboard]                                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ 5. FIRST ANALYSIS (Magic Moment)                                │
│                                                                 │
│    "Have a Benchy or test print? Let's analyze it!"             │
│                                                                 │
│    ┌─────────────────────────────────────────┐                 │
│    │                                         │                 │
│    │     📷 Drop photo here or browse        │                 │
│    │                                         │                 │
│    └─────────────────────────────────────────┘                 │
│                                                                 │
│    [Skip for now]                                               │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│ 6. DASHBOARD TOUR                                               │
│                                                                 │
│    Quick highlights of key features...                          │
│    [Start Using BambuPrint IQ]                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key UX Decisions**:
- Cloud login is recommended (most features, easiest)
- Users without printers can still use profile generator
- Clear explanation of what each auth method enables
- Developer Mode has warning about trade-offs

### Key Screens

#### Dashboard (Home)
- Printer status cards (live)
- Recent prints with quality scores
- Environmental alerts
- Quick actions: "Analyze Print", "New Profile"

#### Analysis View
- Photo upload area (drag/drop)
- AI analysis results with visual annotations
- Recommended changes (diff view)
- "Generate Profile" CTA

#### Profile Manager
- List of profiles (user + community)
- Filter by printer, filament type
- Edit/clone/delete
- Export to Bambu Studio

#### Print Monitor
- Live camera feed
- Real-time metrics graphs
- Alert configuration
- Timelapse player

---

## Success Metrics

### North Star Metric
**Print Success Rate Improvement**: % increase in successful prints after using recommended profiles

### Supporting Metrics

| Metric | Target (6 months) | Measurement |
|--------|-------------------|-------------|
| Weekly Active Users | 10,000 | Analytics |
| Profiles Generated | 50,000 | Database |
| Analysis Accuracy | 85%+ | User feedback/corrections |
| Community Profiles Shared | 5,000 | Database |
| Avg. Quality Score Improvement | +1.5 points | Before/after analysis |
| Print Failure Detection Rate | 90% | Caught before completion |

---

## Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| SDK application denied | Medium | High | Fall back to Cloud API auth + Developer Mode support; core MVP works without printer connection |
| Bambu further restricts API access | Low | High | Already adapted to Jan 2025 changes; support all three auth methods; local-first architecture |
| Cloud API rate limiting | Medium | Medium | Cache aggressively; batch requests; local-first where possible |
| AI analysis accuracy insufficient | Medium | Medium | Hybrid approach: AI + rule engine + user corrections |
| Legal issues with web scraping | Low | Medium | Respect robots.txt, cache results, partner with manufacturers |
| User adoption friction (auth complexity) | Medium | Medium | Clear onboarding with auth method comparison; Cloud login as recommended default |
| Competition from Bambu native features | Medium | Medium | Move faster, focus on AI-powered analysis (not in Bambu's roadmap) |
| Developer Mode users lose cloud features | Low | Low | Clearly communicate trade-offs; most users will use Cloud API |
| Token expiration/refresh issues | Medium | Low | Automatic token refresh; graceful re-auth prompts |

---

## Roadmap

### Phase 1: MVP (Months 1-3)
- [ ] **Apply for Bambu SDK access** (Week 1)
- [ ] Benchy analysis engine (AI-powered)
- [ ] Basic filament spec scraper (top 20 brands)
- [ ] Profile generator (JSON export)
- [ ] Simple web UI for upload/download
- [ ] Read-only MQTT telemetry (no auth required)

### Phase 2: Connected (Months 4-6)
- [ ] **Bambu Auth Bridge implementation**
  - [ ] Cloud API authentication (2FA flow)
  - [ ] Bambu Connect/SDK integration (if approved)
  - [ ] Developer Mode support
- [ ] Full MQTT integration (authenticated)
- [ ] Real-time dashboard with auth-aware features
- [ ] Camera frame capture (auth required)
- [ ] Environmental data display
- [ ] Desktop app (Tauri - Rust-based, lighter than Electron)

### Phase 3: Intelligent (Months 7-9)
- [ ] Multi-printer support (multi-account)
- [ ] Print history tracking
- [ ] Predictive quality scoring
- [ ] Automated profile recommendations
- [ ] In-print defect detection via camera

### Phase 4: Community (Months 10-12)
- [ ] Profile sharing platform
- [ ] Rating/verification system
- [ ] Manufacturer partnerships
- [ ] Mobile app (React Native)
- [ ] Print farm management features

---

## Open Questions

1. **Monetization**: Freemium? Pro tier for farms? Affiliate with filament brands?
2. **Scope**: Bambu-only or expand to other brands (Prusa, Creality)?
3. **Offline Mode**: How much functionality works without internet? (Cloud API requires internet; Bambu Connect/Dev Mode work locally)
4. **Data Privacy**: Where are print photos stored? User control? GDPR compliance?
5. ~~**Developer Mode**: Should we require/recommend Bambu's Developer Mode?~~ **RESOLVED**: Support all three auth methods; recommend Cloud API for most users
6. **SDK Timeline**: When should we apply for Bambu SDK access? (Recommend: Early Phase 1)
7. **Bambu Connect Bundling**: Can we bundle the Local SDK binary, or require users to install Bambu Connect separately?
8. **Multi-Account Support**: Should we support multiple Bambu accounts (e.g., personal + work)?

---

## Appendix

### A. Useful Resources

**Official Bambu Lab**:
- [Third-Party Integration Wiki](https://wiki.bambulab.com/en/software/third-party-integration)
- [Bambu Connect Announcement](https://blog.bambulab.com/updates-and-third-party-integration-with-bambu-connect/)
- [Authorization Control System](https://blog.bambulab.com/firmware-update-introducing-new-authorization-control-system-2/)
- [Profile Format Documentation](https://wiki.bambulab.com/en/bambu-studio/export-filament)

**Community Libraries & Documentation**:
- [Bambu-Lab-Cloud-API (Python)](https://github.com/coelacant1/Bambu-Lab-Cloud-API) - Cloud API, MQTT, video, compatibility layer
- [OpenBambuAPI (Protocol Docs)](https://github.com/Doridian/OpenBambuAPI) - MQTT, FTP, video, TLS documentation
- [bambulabs-api on PyPI](https://pypi.org/project/bambulabs-api/) - Python library
- [bambulab-rs (Rust)](https://github.com/m1guelpf/bambulab-rs) - Unofficial Rust client

**Home Automation & Monitoring**:
- [Home Assistant Bambu Integration](https://community.home-assistant.io/t/bambu-lab-x1-x1c-mqtt/489510)
- [Prometheus/Grafana Monitoring](https://medium.com/@smbaker/monitoring-my-bambu-lab-3d-printer-with-prometheus-and-grafana-b62680e61394)

**AI/ML for 3D Printing**:
- [Obico AI Failure Detection](https://www.obico.io/blog/ai-failure-detection-in-3d-printing/)
- [ORNL Peregrine Software](https://www.ornl.gov/news/ai-software-enables-real-time-3d-printing-quality-assessment)

### B. Competitor Analysis

| Product | Strengths | Weaknesses | Our Differentiator |
|---------|-----------|------------|-------------------|
| Obico | AI failure detection, multi-platform | Generic, not Bambu-optimized | Bambu-native integration |
| OctoEverywhere | Remote access, alerts | Requires OctoPrint | Direct MQTT, no middleman |
| Bambu Handy | Official app, reliable | Limited features, no AI | Intelligence layer |
| Spoolman | Filament tracking | No printer integration | Full ecosystem |

### C. Profile JSON Schema Reference

```json
{
  "$schema": "bambu-filament-v1",
  "filament_id": "string (starts with GF)",
  "setting_id": "string (starts with GFS)",
  "name": "string",
  "inherits": "string (parent profile)",
  "instantiation": "boolean",
  "compatible_printers": ["array of printer strings"],
  "nozzle_temperature": [215],
  "nozzle_temperature_range_low": [190],
  "nozzle_temperature_range_high": [230],
  "bed_temperature": [60],
  "fan_min_speed": [35],
  "fan_max_speed": [100],
  "overhang_fan_speed": [100],
  "slow_down_layer_time": [8],
  "filament_retraction_length": [0.8],
  "filament_retraction_speed": [30],
  "filament_flow_ratio": [0.98]
}
```

---

**Document History**

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-02-04 | Michael Curtis & Claude | Initial draft |
| 1.1 | 2026-02-04 | Michael Curtis & Claude | Added January 2025 Authorization System section; Bambu Connect integration strategy; Updated architecture with Auth Bridge; Revised onboarding flow for multi-auth; Added auth-aware code examples; Updated risks and roadmap |

---

*Let's build something that makes every print better than the last.* 🚀
