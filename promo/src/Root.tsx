import React from 'react';
import {Composition} from 'remotion';
import {LeadlinePromo} from './remotion/Leadline/LeadlinePromo';

export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition
        id="LeadlinePromo"
        component={LeadlinePromo}
        durationInFrames={900}
        fps={30}
        width={1920}
        height={1080}
      />
    </>
  );
};
