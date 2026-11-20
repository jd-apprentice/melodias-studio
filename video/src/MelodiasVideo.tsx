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
import {useAudioData, visualizeAudio} from '@remotion/media-utils';

export type MelodiasVideoProps = {
  durationSec: number;
  channel: string;
};

export const MelodiasVideo: React.FC<MelodiasVideoProps> = ({channel}) => {
  const frame = useCurrentFrame();
  const {fps, durationInFrames} = useVideoConfig();
  const audioData = useAudioData(staticFile('current/audio.mp3'));
  const samples = audioData
    ? visualizeAudio({audioData, frame, fps, numberOfSamples: 64, smoothing: true})
    : new Array(64).fill(0);

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

      <AbsoluteFill style={{mixBlendMode: 'screen', opacity: 0.55}}>
        <Loop durationInFrames={Math.round(45.42 * fps)}>
          <OffthreadVideo
            src={staticFile('overlays/particles-bokeh.mp4')}
            muted
            style={{width: '100%', height: '100%', objectFit: 'cover'}}
          />
        </Loop>
      </AbsoluteFill>

      <AbsoluteFill style={{mixBlendMode: 'screen', opacity: 0.45}}>
        <Loop durationInFrames={Math.round(49.47 * fps)}>
          <OffthreadVideo
            src={staticFile('overlays/particles-sparks.mp4')}
            muted
            style={{width: '100%', height: '100%', objectFit: 'cover'}}
          />
        </Loop>
      </AbsoluteFill>

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

      <AbsoluteFill style={{justifyContent: 'flex-end', padding: '0 70px 40px'}}>
        <div style={{display: 'flex', alignItems: 'flex-end', gap: 6, height: 130}}>
          {samples.map((value, index) => (
            <div
              key={index}
              style={{
                flex: 1,
                height: `${Math.max(2, Math.min(1, Math.pow(value, 0.65)) * 100)}%`,
                background: '#56d4dd',
                opacity: 0.62,
                borderRadius: 2,
              }}
            />
          ))}
        </div>
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
