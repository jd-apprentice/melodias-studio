import React from 'react';
import {
  AbsoluteFill,
  Audio,
  Img,
  Loop,
  OffthreadVideo,
  interpolate,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from 'remotion';

export type MelodiasVideoProps = {
  durationSec: number;
  channel: string;
  particles?: 'both' | 'bokeh' | 'sparks';
};

export const MelodiasVideo: React.FC<MelodiasVideoProps> = ({
  channel,
  particles = 'both',
}) => {
  const showBokeh = particles !== 'sparks';
  const showSparks = particles !== 'bokeh';
  const frame = useCurrentFrame();
  const {fps, durationInFrames} = useVideoConfig();

  const zoom = interpolate(frame, [0, durationInFrames], [1.03, 1.1], {
    extrapolateRight: 'clamp',
  });
  const fadeIn = interpolate(frame, [0, fps * 1.5], [0, 1], {
    extrapolateRight: 'clamp',
  });
  const fadeOut = interpolate(
    frame,
    [durationInFrames - fps * 3, durationInFrames],
    [1, 0],
    {extrapolateLeft: 'clamp'},
  );

  return (
    <AbsoluteFill style={{backgroundColor: 'black'}}>
      <AbsoluteFill
        style={{
          transform: `scale(${zoom})`,
          filter: 'saturate(1.06) contrast(1.03)',
        }}
      >
        <Img
          src={staticFile('current/thumbnail.png')}
          style={{width: '100%', height: '100%', objectFit: 'cover'}}
        />
      </AbsoluteFill>

      {showBokeh ? (
        <AbsoluteFill style={{mixBlendMode: 'screen', opacity: 0.55}}>
          <Loop durationInFrames={Math.round(45.42 * fps)}>
            <OffthreadVideo
              src={staticFile('overlays/particles-bokeh.mp4')}
              muted
              style={{width: '100%', height: '100%', objectFit: 'cover'}}
            />
          </Loop>
        </AbsoluteFill>
      ) : null}

      {showSparks ? (
        <AbsoluteFill style={{mixBlendMode: 'screen', opacity: 0.45}}>
          <Loop durationInFrames={Math.round(49.47 * fps)}>
            <OffthreadVideo
              src={staticFile('overlays/particles-sparks.mp4')}
              muted
              style={{width: '100%', height: '100%', objectFit: 'cover'}}
            />
          </Loop>
        </AbsoluteFill>
      ) : null}

      <AbsoluteFill
        style={{
          mixBlendMode: 'screen',
          opacity: 0.32,
          clipPath: 'inset(0 0 35% 0)',
        }}
      >
        <Loop durationInFrames={Math.round(10.73 * fps)}>
          <OffthreadVideo
            src={staticFile('overlays/birds.mp4')}
            muted
            style={{width: '100%', height: '100%', objectFit: 'cover'}}
          />
        </Loop>
      </AbsoluteFill>

      <AbsoluteFill
        style={{
          background:
            'linear-gradient(to top, rgba(0,0,0,0.6), rgba(0,0,0,0) 24%)',
        }}
      />

      <AbsoluteFill style={{mixBlendMode: 'screen', opacity: 0.6}}>
        <OffthreadVideo
          src={staticFile('current/waveform.mp4')}
          muted
          style={{width: '100%', height: '100%', objectFit: 'cover'}}
        />
      </AbsoluteFill>

      {channel ? (
        <div
          style={{
            position: 'absolute',
            right: 56,
            top: 40,
            color: '#f0f6fc',
            opacity: 0.4,
            fontFamily: 'monospace',
            fontSize: 24,
            letterSpacing: 3,
          }}
        >
          {channel}
        </div>
      ) : null}

      <AbsoluteFill
        style={{backgroundColor: 'black', opacity: 1 - Math.min(fadeIn, fadeOut)}}
      />
      <Audio src={staticFile('current/audio.mp3')} />
    </AbsoluteFill>
  );
};
