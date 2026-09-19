export function Loader() {
  return (
    <div className="uib-dot-loader" role="status" aria-label="Loading">
      <div className="uib-dot" />
      <div className="uib-dot" />
      <div className="uib-dot" />
      <div className="uib-dot" />
      <div className="uib-dot" />
      <div className="uib-dot" />
      <span className="sr-only">Loading...</span>

      <style>{`
        .uib-dot-loader {
          position: relative;
          display: flex;
          align-items: center;
          justify-content: flex-start;
          height: 40px;
          width: 40px;
          animation: uib-smoothRotate 1.80s linear infinite;
        }

        .uib-dot-loader .uib-dot {
          position: absolute;
          top: 0;
          left: 0;
          display: flex;
          align-items: flex-start;
          justify-content: center;
          height: 100%;
          width: 100%;
          animation: uib-rotate 1s ease-in-out infinite;
        }

        .uib-dot-loader .uib-dot::before {
          content: "";
          height: 6.80px;
          width: 6.80px;
          border-radius: 50%;
          background-color: #167c51;
          transition: background-color 0.3s ease;
        }

        .uib-dot-loader .uib-dot:nth-child(2) {
          animation-delay: -0.417s;
        }
        .uib-dot-loader .uib-dot:nth-child(3) {
          animation-delay: -0.334s;
        }
        .uib-dot-loader .uib-dot:nth-child(4) {
          animation-delay: -0.251s;
        }
        .uib-dot-loader .uib-dot:nth-child(5) {
          animation-delay: -0.167s;
        }
        .uib-dot-loader .uib-dot:nth-child(6) {
          animation-delay: -0.084s;
        }

        @keyframes uib-rotate {
          0% {
            transform: rotate(0deg);
          }
          65%,
          100% {
            transform: rotate(360deg);
          }
        }

        @keyframes uib-smoothRotate {
          0% {
            transform: rotate(0deg);
          }
          100% {
            transform: rotate(360deg);
          }
        }
      `}</style>
    </div>
  )
}
