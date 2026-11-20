import {Composition} from 'remotion';
import {MelodiasVideo, type MelodiasVideoProps} from './MelodiasVideo';

export const RemotionRoot = () => {
  return (
    <Composition<MelodiasVideoProps>
      id="MelodiasVideo"
      component={MelodiasVideo}
      durationInFrames={30 * 210}
      fps={30}
      width={1920}
      height={1080}
      defaultProps={{durationSec: 210, channel: 'melodias8bit'}}
      calculateMetadata={({props}) => ({
        durationInFrames: Math.max(1, Math.round(props.durationSec * 30)),
      })}
    />
  );
};
