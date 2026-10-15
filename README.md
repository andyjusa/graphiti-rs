<p align="center">
  <img src="docs/assets/readme-banner.svg" alt="graphiti-rs — Temporal memory for a small, always-on machine" width="100%">
</p>

<p align="center">
  <a href="https://github.com/andyjusa/graphiti-rs/actions/workflows/ci.yml"><img src="https://github.com/andyjusa/graphiti-rs/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-2024_edition-ea9a65?logo=rust&amp;logoColor=white" alt="Rust 2024 edition"></a>
  <a href="docs/compat/pi-results.md"><img src="https://img.shields.io/badge/Raspberry_Pi-ARM64-c95579?logo=raspberrypi&amp;logoColor=white" alt="Raspberry Pi ARM64"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-8acbb1" alt="Apache-2.0 license"></a>
</p>

<p align="center">
  <strong>라즈베리파이에 올려 두는 나만의 그래프 메모리 서버.</strong><br>
  Rust · FalkorDB · MCP · 선택적 로컬 임베딩
</p>

<p align="center">
  <a href="#빠른-시작">빠른 시작</a> ·
  <a href="#기존-graphiti와의-차이">Graphiti와 비교</a> ·
  <a href="#raspberry-pi-실측">Pi 실측</a> ·
  <a href="docs/standalone-guide.md">설정·API 가이드</a>
</p>

## 작은 서버에서 오래 켜 두기 위해

**graphiti-rs는 제 라즈베리파이에서 Graphiti 메모리 서버를 운영하기 위해 만든 개인 프로젝트입니다.**
[getzep/graphiti](https://github.com/getzep/graphiti)의 시간 기반 그래프 메모리 설계를 참고해, 필요한 범위를 Rust로 독립 구현했습니다. 하나의 Rust 서버 프로세스와 FalkorDB를 중심으로 구성하고, 동시 작업·검색 후보·요청 시간을 제한합니다.

에피소드 원문에서 엔티티와 사실을 연결하고, 사실이 바뀌면 이전 기록과 출처를 남깁니다. MCP 클라이언트는 stdio 또는 Streamable HTTP로 메모리를 저장하고 검색할 수 있습니다. 자동 추출에는 OpenAI 호환 API를, 임베딩에는 외부 API나 선택적 로컬 CPU 추론을 사용할 수 있습니다. 구조화된 추출 결과를 직접 전달하면 모델 API 없이도 시작할 수 있습니다.

> [!NOTE]
> 공식 Graphiti의 Rust 버전이 아닌 개인의 독립 구현입니다. Python API 전체, 모든 데이터베이스 백엔드, 최신 MCP 기능과의 호환을 약속하지 않습니다. 기존 Graphiti 데이터를 사용하려면 아래의 **호환 모드**를 확인하세요.

## Raspberry Pi 실측

<!-- PAIRED_BENCHMARK -->
**같은 Pi4에서 서버 메모리 약 89% 감소: Python 118.31 MiB → Rust 13.16 MiB RSS.**

2026-10-15, 실제 Raspberry Pi4에서 기존 **Graphiti 0.28.2 MCP 서버**와 **배포된 Rust 릴리스 `8df9904`**를 비교했습니다. 같은 DB·합성 사실 1,200개·3072차원 mock 임베딩·1 CPU 조건에서 순서를 번갈아 3회 측정했습니다. Mac에서는 벤치마크를 실행하지 않았습니다.

| 측정 항목 | Python Graphiti + MCP | graphiti-rs |
| --- | ---: | ---: |
| 조회·검색 후 서버 RSS · 3회 중앙값 | 118.31 MiB | **13.16 MiB** |
| 서버 프로세스 peak RSS · 3회 최댓값 | 118.32 MiB | **13.58 MiB** |
| 서버 CPU · 300요청당 중앙값 | 28.33초 | **2.09초** |
| 원문 조회 p50 | 24.12 ms | **2.62 ms** |
| 하이브리드 사실 검색 p50 | 152.90 ms | **48.07 ms** |
| 의미 기반 노드 검색 p50 | 123.72 ms | **26.14 ms** |

p50은 각 회차의 100요청 p50을 다시 중앙값으로 집계했습니다. 작업별 10회 warmup은 제외했습니다. 측정 요청 **1,800건 모두 성공**했고 양쪽의 전체 tool 응답은 시간 표기를 정규화한 뒤 해시가 일치했습니다.

**위 메모리 수치는 서버 프로세스만의 값입니다.** FalkorDB는 별도로 각 회차 완료 시 약 71.8–75.9 MiB RSS를 사용했습니다. 모델 추론·수집 처리량·최신 Python 0.31의 메모리를 측정한 결과는 아닙니다. DB와 모델을 포함한 전체 시스템의 89% 절감을 뜻하지 않습니다.

[측정 조건·회차별 범위·재현 방법](docs/benchmarks/pi-paired-20261015.md) · [원본 측정 JSON](docs/benchmarks/pi-paired-20261015.json) · [집계·소스 해시](docs/benchmarks/pi-paired-20261015-summary.json)
<!-- /PAIRED_BENCHMARK -->

지금 확인할 수 있는 근거는 [Pi4 ARM64 검증](docs/compat/pi-results.md), [벡터 검색 측정](docs/compat/vector-retrieval.md), [운영 전환 기록](docs/compat/operational-cutover-20261003.md)입니다. 각 기록의 데이터와 실행 모드는 다르며, 서버 RSS와 FalkorDB·모델을 포함한 전체 메모리 사용량은 구분해야 합니다.


## 기존 Graphiti와의 차이

비교 기준은 [Graphiti 0.31.0 소스](https://github.com/getzep/graphiti/tree/aca369364226a666ca541b061ff2a1a54b80f2ff)입니다. 기능의 폭보다 **Pi에서 필요한 메모리 서버를 운영하는 범위**에 집중합니다.

| | Graphiti · Python | graphiti-rs · Rust |
| --- | --- | --- |
| 목표 | 시간 기반 지식 그래프를 구축하는 범용 프레임워크·SDK | 개인 Raspberry Pi용 메모리·MCP 서버 |
| 실행 구성 | Python 3.10+와 선택한 DB·모델 제공자 | 컴파일된 Rust 서버와 FalkorDB; 서버에 Python 런타임 불필요 |
| 그래프 저장소 | Neo4j, FalkorDB, Neptune 등 여러 백엔드 | FalkorDB에 집중 |
| 기능 범위 | 커스텀 타입, 커뮤니티, 다양한 검색·모델 구성 | 에피소드·엔티티·사실·유효 기간·출처와 제한된 검색·수집 |
| 모델 구성 | 외부 제공자 및 선택적 로컬 모델 통합 | 외부 OpenAI 호환 API; 로컬 임베딩은 빌드 기능으로 선택 |
| 호환성 기준 | 업스트림의 Python·MCP 계약 | 독립 모드 기본 6도구; 별도 기능으로 관찰된 0.28.2 MCP 10도구 |

Python 측 로컬 모델 의존성도 선택 사항입니다. 메모리·CPU·응답 시간은 위의 [동일 Pi 실측](#raspberry-pi-실측)으로 비교합니다. 최신 업스트림의 일부 정확성 수정은 선별 반영했습니다. [반영 내역과 의도적으로 남긴 차이](docs/compat/upstream-main-20261015.md)를 공개합니다.

## 두 가지 실행 모드

| | 독립 모드 · 기본 | Graphiti 호환 모드 · 선택 |
| --- | --- | --- |
| 용도 | 새 메모리 저장소 시작 | 관찰된 기존 Graphiti 배포와 연동 |
| 빌드 | 기본 빌드, 선택적으로 `local-embeddings` | `--features graphiti-compat` |
| 저장 형식 | 자체 스키마·별도 그래프 이름 | 관찰된 Graphiti 0.28.2 스키마 |
| MCP | 기본 6도구 | 기록된 10도구 |
| 쓰기·모델 | 명시적 추출 또는 외부 API; 임베딩 선택 | 기본 읽기 전용; operational 쓰기는 검증된 제공자·3072차원 프로필·영속 저널 필요 |

독립 모드의 6도구는 `add_episode`, `search_facts`, `search_nodes`, `get_episode`, `delete_episode`, `get_status`입니다. 도구 인자와 시간·삭제 의미는 [API 가이드](docs/standalone-guide.md#mcp-tools)에 정리했습니다. 기본 단일 토큰은 모든 그룹에 접근하므로 그룹 ID를 사용자별 권한 경계로 사용하면 안 됩니다.

호환 모드는 최신 Graphiti 전체를 그대로 대체하지 않습니다. 기존 임베딩 공간을 로컬 모델로 바꾸는 기능도 아닙니다. [호환 범위](docs/compat/implementation.md), [운영 설정](docs/compat/operational-mode.md), [Pi 전환 기록](docs/compat/operational-cutover-20261003.md)을 먼저 확인하세요.

## 빠른 시작

새 저장소를 만드는 **독립 모드** 예제입니다. Docker Compose와 데모용 Python 3가 필요합니다.

```sh
git clone https://github.com/andyjusa/graphiti-rs.git
cd graphiti-rs
cp .env.example .env
# .env의 GRAPHITI_TOKEN을 충분히 긴 임의 값으로 변경
docker compose up -d --build

# .env에 지정한 토큰과 같은 값
export GRAPHITI_TOKEN='your-token-from-.env'
python3 examples/demo.py
```

Compose는 `127.0.0.1:8080/mcp`와 로컬 FalkorDB를 띄우고, DB 데이터를 named volume에 저장하며 AOF를 켭니다. 데모는 명시적 추출과 임베딩 비활성화를 사용하므로 유료 모델 API가 필요 없습니다. 서버 자체에는 Python 런타임이 필요하지 않습니다.

기존 FalkorDB를 사용할 때는 **Python Graphiti와 다른 그래프 이름**으로 시작합니다.

```sh
cargo build --release --locked
export FALKORDB_URL=redis://127.0.0.1:6379
export GRAPHITI_GRAPH=graphiti_rs
./target/release/graphiti-rs --transport stdio
# HTTP로 실행하려면: --transport http
```

직접 실행한 바이너리는 `.env`를 자동으로 읽지 않습니다. HTTP 인증, 역방향 프록시, 작업 한도와 TLS 설정은 [운영·리소스 가이드](docs/standalone-guide.md#resource-defaults)를 확인하세요. Rust 1.95로 검증했으며, manifest의 최소 버전 1.88은 별도로 검증하지 않았습니다.

### 로컬 임베딩은 필요할 때만

기본 빌드에는 Candle·tokenizer 의존성이 없습니다. 로컬 임베딩을 쓰려면 모델을 미리 준비하고 기능을 켭니다.

```sh
python3 scripts/download-minilm.py
cargo build --release --locked --features local-embeddings
export EMBEDDING_PROVIDER=local
export EMBEDDING_MODEL=sentence-transformers/all-MiniLM-L6-v2
export EMBEDDING_DIM=384
export LOCAL_MODEL_DIR="$PWD/models/minilm"
./target/release/graphiti-rs --transport stdio
```

한 CPU worker가 순차 추론하며, 실행 중 모델을 내려받지 않습니다. 예제 MiniLM은 영어 모델입니다. 한국어에는 적합한 다국어 모델을 별도로 검증해야 합니다. 모델 형식, 그룹별 임베딩 고정과 Compose 설정은 [로컬 임베딩 가이드](docs/standalone-guide.md#optional-local-embeddings)에 있습니다.


## 검증과 문서

ARM64 Pi의 격리 테스트와 로컬 CPU 임베딩 추론을 검증했고, 별도 호환 프로필의 기존 데이터 전환·재시작 결과도 기록했습니다. CI는 선택적 로컬 추론과 Linux ARM64 빌드를 확인합니다. ARMv7, 범용 모델 품질, 모든 업스트림 기능의 동등성은 검증 범위에 포함하지 않습니다.

| 문서 | 내용 |
| --- | --- |
| [독립 서버 가이드](docs/standalone-guide.md) | 전체 설정, 6도구 API, 시간·삭제 계약, 임베딩, 리소스 한도, 검사 명령 |
| [호환 구현](docs/compat/implementation.md) · [운영 설정](docs/compat/operational-mode.md) | 10도구 프로필, 기존 스키마, 권한, 제공자·저널 요건 |
| [업스트림 비교](docs/compat/upstream-main-20261015.md) | 0.31.0 선별 반영과 제외 범위 |
| [검증 기록](docs/verification.md) · [Pi 근거](docs/compat/pi-results.md) | 실제 실행 결과와 검증 한계 |

## 라이선스·크레딧

[Apache-2.0](LICENSE). [Graphiti](https://github.com/getzep/graphiti)의 설계와 공개 구현에 감사드립니다. 이 저장소는 독립 Rust 구현이며, 호환 기능에 포함된 업스트림 프롬프트 등의 출처는 [출처 고지](tests/fixtures/python-0282/NOTICE)에 표시합니다. 내려받은 모델은 각 모델의 라이선스를 따르며 Git에 포함하지 않습니다.
