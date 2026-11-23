# RenderPRO — novidades

## 0.1.0

- Primeira versão do RenderPRO / First RenderPRO release
- Arrastar vídeos ou pastas → RENDER → escolher o player: PLAYERSLICE, VIDEOHOST PLAYER ou Personalizado
- Modo automático: o codec recomendado para cada vídeo (HAP, HAP Alpha se tiver transparência, H.264 no VIDEOHOST até 1080p60)
- Codecs: HAP, HAP Q, HAP Alpha, H.264, H.265, ProRes 422 HQ, ProRes 4444, DNxHR HQ
- HAP com chunks (4–8 em resoluções grandes) e tamanho em múltiplos de 4; som PCM 48 kHz intercalado dentro do .mov
- Resolução original, metade ou personalizada (esticar, caber, preencher); frame rate constante; desentrelaçamento automático
- Normalização de volume EBU R128 (opcional)
- Fiabilidade: escreve num ficheiro temporário e só entrega depois de verificado (codec, tamanho, duração, som e descodificação completa); verifica o espaço em disco; nunca altera nem substitui o original
- Português e inglês; atualizações automáticas a partir do GitHub
